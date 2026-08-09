//! Cover art: where to look, and in what order.
//!
//! Different editions carry different covers, which is exactly why art is worth
//! fetching — it is how a person recognises the pressing they meant at a glance.
//! It is also never worth waiting for, so everything here describes *where* to
//! look and nothing here blocks: `ui::art` fetches lazily, and a missing cover
//! renders a placeholder rather than an error.
//!
//! The order below is by how well the source lines up with what has already been
//! resolved, not by picture quality. The Cover Art Archive is keyed by MBID, so
//! it is the only source that can be *certain* it is showing the right edition;
//! the rest are text matches on an album that may or may not be the same
//! pressing, and are fallbacks for that reason.

use serde_json::Value;

use super::{parse_json, Outcome, Reason, Request, SourceId};
use crate::model::album::Mbid;

/// The sizes the Cover Art Archive renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    /// List rows.
    Thumbnail,
    /// The detail pane.
    Full,
}

impl Size {
    pub fn pixels(self) -> u32 {
        match self {
            Size::Thumbnail => 250,
            Size::Full => 500,
        }
    }
}

/// Front cover for one specific release.
pub fn cover_art_archive_release(release: &Mbid, size: Size) -> Request {
    Request::get(
        SourceId::CoverArtArchive,
        format!(
            "https://coverartarchive.org/release/{}/front-{}",
            release.as_str(),
            size.pixels()
        ),
    )
}

/// Front cover for a release group, when no specific release is chosen yet.
pub fn cover_art_archive_group(group: &Mbid, size: Size) -> Request {
    Request::get(
        SourceId::CoverArtArchive,
        format!(
            "https://coverartarchive.org/release-group/{}/front-{}",
            group.as_str(),
            size.pixels()
        ),
    )
}

/// Deezer's public album search, the last resort.
pub fn deezer_search(artist: &str, album: &str) -> Request {
    Request::get(
        SourceId::Deezer,
        format!(
            "https://api.deezer.com/search/album?q={}&limit=1",
            encode(format!("{artist} {album}").trim())
        ),
    )
    .header("Accept", "application/json")
}

pub fn parse_deezer_cover(body: &[u8], size: Size) -> Outcome<String> {
    let root = match parse_json(SourceId::Deezer, body) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };
    let Some(items) = root.get("data").and_then(Value::as_array) else {
        return Outcome::Stale(Reason::Malformed("no data array".into()));
    };
    let Some(first) = items.first() else {
        return Outcome::Empty;
    };

    let key = match size {
        Size::Thumbnail => "cover_medium",
        Size::Full => "cover_big",
    };
    match first
        .get(key)
        .and_then(Value::as_str)
        .or_else(|| first.get("cover").and_then(Value::as_str))
    {
        Some(url) if !url.is_empty() => Outcome::Found(url.to_string()),
        _ => Outcome::Empty,
    }
}

/// Where to look for one album's cover, best-matched source first.
///
/// A caller walks this in order and stops at the first image that arrives. The
/// iTunes step is expressed as an already-resized URL rather than a request,
/// because the iTunes source has usually already been called for the price and
/// its artwork URL comes along for free — refetching it would spend one of a
/// tight request budget on a picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attempt {
    CoverArtArchive(Request),
    /// A ready-to-fetch URL, from iTunes or Bandcamp.
    Direct(String),
    Deezer(Request),
}

/// Build the fallback chain for a release.
pub fn chain(
    release: Option<&Mbid>,
    group: Option<&Mbid>,
    itunes_artwork: Option<&str>,
    bandcamp_art: Option<&str>,
    artist: &str,
    album: &str,
    size: Size,
) -> Vec<Attempt> {
    let mut attempts = Vec::new();

    // MBID-keyed, so it is the only source that cannot be showing a different
    // edition's sleeve. Release before group: the group's cover is whichever one
    // an editor picked as representative, which may not be the pressing chosen.
    if let Some(release) = release {
        attempts.push(Attempt::CoverArtArchive(cover_art_archive_release(
            release, size,
        )));
    }
    if let Some(group) = group {
        attempts.push(Attempt::CoverArtArchive(cover_art_archive_group(
            group, size,
        )));
    }
    if let Some(url) = itunes_artwork {
        attempts.push(Attempt::Direct(super::itunes::artwork_at(
            url,
            size.pixels(),
        )));
    }
    if let Some(url) = bandcamp_art {
        attempts.push(Attempt::Direct(url.to_string()));
    }
    if !artist.is_empty() || !album.is_empty() {
        attempts.push(Attempt::Deezer(deezer_search(artist, album)));
    }

    attempts
}

fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_archive_is_tried_before_any_text_matched_source() {
        // Only the MBID-keyed source is certain to be the right edition. If a
        // text match ever came first, a search for one pressing could show
        // another's sleeve, which defeats the purpose of showing art at all.
        let chain = chain(
            Some(&Mbid::new("r1")),
            Some(&Mbid::new("g1")),
            Some("https://is1.mzstatic.com/x/100x100bb.jpg"),
            Some("https://f4.bcbits.com/img/a1.jpg"),
            "Radiohead",
            "Kid A",
            Size::Thumbnail,
        );
        assert!(matches!(chain[0], Attempt::CoverArtArchive(_)));
        assert!(matches!(chain[1], Attempt::CoverArtArchive(_)));
        assert!(matches!(chain[2], Attempt::Direct(_)));
        assert!(matches!(chain[4], Attempt::Deezer(_)));
    }

    #[test]
    fn the_specific_release_is_tried_before_the_group() {
        let chain = chain(
            Some(&Mbid::new("r1")),
            Some(&Mbid::new("g1")),
            None,
            None,
            "",
            "",
            Size::Full,
        );
        let Attempt::CoverArtArchive(first) = &chain[0] else {
            panic!("expected the archive first");
        };
        assert!(first.url.contains("/release/r1/front-500"));
        let Attempt::CoverArtArchive(second) = &chain[1] else {
            panic!("expected the archive second");
        };
        assert!(second.url.contains("/release-group/g1/front-500"));
    }

    #[test]
    fn the_itunes_link_is_upsized_rather_than_used_at_its_thumbnail_size() {
        let chain = chain(
            None,
            None,
            Some("https://is1.mzstatic.com/x/100x100bb.jpg"),
            None,
            "",
            "",
            Size::Full,
        );
        assert_eq!(
            chain[0],
            Attempt::Direct("https://is1.mzstatic.com/x/500x500bb.jpg".into())
        );
    }

    #[test]
    fn nothing_known_yields_an_empty_chain_rather_than_a_useless_request() {
        assert!(chain(None, None, None, None, "", "", Size::Thumbnail).is_empty());
    }

    #[test]
    fn a_deezer_response_gives_the_size_that_was_asked_for() {
        let body = br#"{"data":[{"id":1,"title":"Kid A",
            "cover":"https://e-cdns.dzcdn.net/x/cover.jpg",
            "cover_medium":"https://e-cdns.dzcdn.net/x/250.jpg",
            "cover_big":"https://e-cdns.dzcdn.net/x/500.jpg"}]}"#;
        assert_eq!(
            parse_deezer_cover(body, Size::Thumbnail),
            Outcome::Found("https://e-cdns.dzcdn.net/x/250.jpg".into())
        );
        assert_eq!(
            parse_deezer_cover(body, Size::Full),
            Outcome::Found("https://e-cdns.dzcdn.net/x/500.jpg".into())
        );
    }

    #[test]
    fn a_deezer_album_with_no_cover_is_empty_rather_than_a_broken_url() {
        assert_eq!(
            parse_deezer_cover(br#"{"data":[{"id":1,"cover":""}]}"#, Size::Full),
            Outcome::Empty
        );
        assert_eq!(
            parse_deezer_cover(br#"{"data":[]}"#, Size::Full),
            Outcome::Empty
        );
    }
}
