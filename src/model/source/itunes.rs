//! The iTunes Store: purchase availability and price, and a fallback cover.
//!
//! Free, no key, and the only source that confirms a purchase price without a
//! token of any kind. It is also the one source with a fixed answer about
//! format: everything the iTunes Store sells is 256 kbps AAC.
//!
//! The "Apple Digital Masters" badge describes the *encoding workflow* — the
//! master handed to Apple was high resolution — and says nothing about what
//! lands on disk, which is still a 256 kbps lossy file. It is deliberately never
//! read as a hi-res signal here. Doing so would put a lossy purchase in tier A.

use serde_json::Value;

use super::{parse_json, Outcome, Reason, Request, SourceId};
use crate::model::offer::{Acquisition, Delivery, Offer, Vendor};

/// Search the store for albums.
pub fn search_albums(artist: &str, album: &str, country: &str) -> Request {
    let term = format!("{artist} {album}");
    Request::get(
        SourceId::ITunes,
        format!(
            "https://itunes.apple.com/search?term={}&entity=album&country={}&limit=25",
            encode(term.trim()),
            encode(country)
        ),
    )
}

/// One album in the iTunes catalogue.
#[derive(Debug, Clone, PartialEq)]
pub struct Album {
    pub collection_id: u64,
    pub title: String,
    pub artist: String,
    pub track_count: usize,
    pub release_year: Option<i32>,
    pub price: Option<(f64, String)>,
    pub url: Option<String>,
    /// The 100px thumbnail URL, as returned. Resize with [`artwork_at`].
    pub artwork_100: Option<String>,
}

impl Album {
    /// What the iTunes Store sells: a 256 kbps AAC purchase, DRM-free.
    pub fn to_offer(&self) -> Offer {
        let mut offer = Offer::new(
            Vendor::ITunes,
            Acquisition::Purchase,
            Delivery::lossy("AAC 256"),
        );
        if let Some((amount, currency)) = &self.price {
            offer = offer.with_price(*amount, currency);
        }
        if let Some(url) = &self.url {
            offer = offer.with_url(url.clone());
        }
        offer
    }
}

/// Rewrite an artwork URL to a larger square.
///
/// Apple's artwork URLs carry the dimensions in the last path segment, so a
/// 100px thumbnail becomes a 600px cover by editing the string — no second
/// request and no API for it. The segment is replaced rather than pattern-
/// matched loosely, so a URL in an unexpected shape comes back untouched instead
/// of mangled.
pub fn artwork_at(url: &str, size: u32) -> String {
    let Some(slash) = url.rfind('/') else {
        return url.to_string();
    };
    let (head, tail) = url.split_at(slash + 1);
    let Some(first_x) = tail.find('x') else {
        return url.to_string();
    };
    if !tail[..first_x].chars().all(|c| c.is_ascii_digit()) || first_x == 0 {
        return url.to_string();
    }
    // "100x100bb.jpg" -> everything from the second dimension's end onward.
    let rest = &tail[first_x + 1..];
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return url.to_string();
    }
    format!("{head}{size}x{size}{}", &rest[digits..])
}

pub fn parse_albums(body: &[u8]) -> Outcome<Vec<Album>> {
    let root = match parse_json(SourceId::ITunes, body) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };

    let Some(items) = root.get("results").and_then(Value::as_array) else {
        return Outcome::Stale(Reason::Malformed("no results array".into()));
    };

    let albums: Vec<Album> = items
        .iter()
        .filter(|item| {
            // A search for an album term still returns artists and songs when
            // the catalogue has them; only collections are purchasable albums.
            // `map_or(true, ..)` rather than `is_none_or`, which is stable
            // only from 1.82 and this crate's floor is 1.80.
            item.get("wrapperType")
                .and_then(Value::as_str)
                .map_or(true, |kind| kind == "collection")
        })
        .filter_map(|item| {
            let collection_id = item.get("collectionId").and_then(Value::as_u64)?;
            let price = item
                .get("collectionPrice")
                .and_then(Value::as_f64)
                // A negative price is how the store says "album not sold as a
                // whole, tracks only". Not a discount, and not a free album.
                .filter(|amount| *amount >= 0.0)
                .zip(string(item, "currency"));
            Some(Album {
                collection_id,
                title: string(item, "collectionName").unwrap_or_default(),
                artist: string(item, "artistName").unwrap_or_default(),
                track_count: item.get("trackCount").and_then(Value::as_u64).unwrap_or(0) as usize,
                release_year: string(item, "releaseDate")
                    .as_deref()
                    .and_then(|date| date.get(..4).and_then(|year| year.parse().ok())),
                price,
                url: string(item, "collectionViewUrl"),
                artwork_100: string(item, "artworkUrl100"),
            })
        })
        .collect();

    Outcome::of_collection(albums)
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
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
    use crate::model::tier::{tier_of, Tier};

    #[test]
    fn an_itunes_purchase_is_tier_b_however_the_album_is_badged() {
        // "Apple Digital Masters" is a workflow badge on a 256 kbps file. If
        // this ever returns tier A, a lossy purchase is outranking every
        // lossless one in the list.
        let album = Album {
            collection_id: 1,
            title: "Kid A (Apple Digital Master)".into(),
            artist: "Radiohead".into(),
            track_count: 10,
            release_year: Some(2000),
            price: Some((7.99, "GBP".into())),
            url: Some("https://music.apple.com/gb/album/1".into()),
            artwork_100: None,
        };
        let offer = album.to_offer();
        assert_eq!(tier_of(&offer), Tier::B);
        assert!(!offer.delivery.lossless);
        assert_eq!(offer.delivery.describe(), "AAC 256");
    }

    #[test]
    fn artwork_urls_resize_by_rewriting_the_dimension_segment() {
        assert_eq!(
            artwork_at(
                "https://is1-ssl.mzstatic.com/image/thumb/abc/100x100bb.jpg",
                600
            ),
            "https://is1-ssl.mzstatic.com/image/thumb/abc/600x600bb.jpg"
        );
        assert_eq!(
            artwork_at("https://example.com/art/60x60bb-60.png", 1200),
            "https://example.com/art/1200x1200bb-60.png"
        );
    }

    #[test]
    fn an_artwork_url_in_an_unexpected_shape_comes_back_untouched() {
        // Better a small picture than a 404 built by string surgery.
        for url in [
            "https://example.com/cover.jpg",
            "https://example.com/",
            "nonsense",
        ] {
            assert_eq!(artwork_at(url, 600), url);
        }
    }

    #[test]
    fn a_search_response_parses_into_albums_with_prices() {
        let body = br#"{"resultCount":1,"results":[{
            "wrapperType":"collection","collectionId":1109714933,
            "collectionName":"Kid A","artistName":"Radiohead",
            "collectionPrice":7.99,"currency":"GBP","trackCount":10,
            "releaseDate":"2000-10-02T07:00:00Z",
            "collectionViewUrl":"https://music.apple.com/gb/album/kid-a/1109714933",
            "artworkUrl100":"https://is1-ssl.mzstatic.com/image/thumb/x/100x100bb.jpg"}]}"#;
        let Outcome::Found(albums) = parse_albums(body) else {
            panic!("expected albums");
        };
        assert_eq!(albums[0].collection_id, 1109714933);
        assert_eq!(albums[0].price, Some((7.99, "GBP".into())));
        assert_eq!(albums[0].release_year, Some(2000));
    }

    #[test]
    fn a_negative_price_means_tracks_only_and_yields_no_price() {
        // The store returns -1.00 for albums it will not sell whole. Read as a
        // number it would be the cheapest option in the ranking.
        let body = br#"{"results":[{"wrapperType":"collection","collectionId":1,
            "collectionName":"X","artistName":"Y","collectionPrice":-1.00,"currency":"USD"}]}"#;
        let Outcome::Found(albums) = parse_albums(body) else {
            panic!("expected albums");
        };
        assert_eq!(albums[0].price, None);
        assert_eq!(albums[0].to_offer().price, None);
    }

    #[test]
    fn artists_and_tracks_in_the_results_are_not_mistaken_for_albums() {
        let body = br#"{"results":[
            {"wrapperType":"artist","artistId":1,"artistName":"Radiohead"},
            {"wrapperType":"collection","collectionId":2,"collectionName":"Kid A","artistName":"Radiohead"}]}"#;
        let Outcome::Found(albums) = parse_albums(body) else {
            panic!("expected albums");
        };
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].collection_id, 2);
    }

    #[test]
    fn zero_results_is_empty_and_a_changed_shape_is_stale() {
        assert_eq!(
            parse_albums(br#"{"resultCount":0,"results":[]}"#),
            Outcome::Empty
        );
        assert!(matches!(
            parse_albums(br#"{"error":"nope"}"#),
            Outcome::Stale(_)
        ));
    }

    #[test]
    fn the_search_url_carries_the_configured_country() {
        let request = search_albums("Radiohead", "Kid A", "GB");
        assert!(request.url.contains("country=GB"));
        assert!(request.url.contains("entity=album"));
        assert!(request.url.contains("Radiohead%20Kid%20A"));
    }
}
