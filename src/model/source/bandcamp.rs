//! Bandcamp: the highest-payout source there is, and the one most likely to win.
//!
//! There is no documented API, but there is no need to scrape either. Bandcamp's
//! own mobile client talks to two JSON endpoints that answer exactly what this
//! application needs — `fuzzysearch` for finding an album, and `tralbum_details`
//! for its price and whether it is actually for sale. Both answer without a key
//! and without pretending to be a browser. That is a far better footing than
//! parsing their search page, which is what the brief assumed would be needed:
//! JSON has a shape a test can assert on, and HTML has a layout that changes.
//!
//! Undocumented still means unversioned, so both parsers report
//! [`Outcome::Stale`] rather than [`Outcome::Empty`] when the shape is wrong.
//! Bandcamp silently disappearing from a ranking would remove the best option
//! from most of them and leave a confident-looking answer behind.

use serde_json::Value;

use super::{parse_json, Outcome, Reason, Request, SourceId};
use crate::model::offer::{Acquisition, Delivery, Offer, Vendor};

/// Search across artists, albums and tracks.
pub fn search(query: &str) -> Request {
    Request::get(
        SourceId::Bandcamp,
        format!(
            "https://bandcamp.com/api/fuzzysearch/1/app_autocomplete?q={}&param_with_locations=true",
            encode(query)
        ),
    )
    .header("Accept", "application/json")
}

/// Price, availability and format for one album.
pub fn album_details(band_id: u64, album_id: u64) -> Request {
    Request::get(
        SourceId::Bandcamp,
        format!(
            "https://bandcamp.com/api/mobile/24/tralbum_details?band_id={band_id}&tralbum_type=a&tralbum_id={album_id}"
        ),
    )
    .header("Accept", "application/json")
}

/// A search hit.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub album_id: u64,
    pub band_id: u64,
    pub title: String,
    pub artist: String,
    pub url: String,
    /// Bandcamp's own cover image, which arrives free with the search and needs
    /// no second request to any art service.
    pub art: Option<String>,
}

/// What Bandcamp will actually do for you with this album.
#[derive(Debug, Clone, PartialEq)]
pub struct Details {
    pub title: String,
    pub artist: Option<String>,
    pub price: Option<(f64, String)>,
    /// Whether it can be bought at all.
    ///
    /// Not the same as being on Bandcamp. *Kid A* is on Radiohead's page,
    /// carries a price, and returns `false` here — it is streamable but not for
    /// sale. Inferring a purchase from mere presence would invent a tier-A offer
    /// that does not exist and put it top of the ranking.
    pub purchasable: bool,
    pub downloadable_tracks: u64,
    pub url: Option<String>,
}

impl Details {
    /// The tier-A offer, when there is one.
    ///
    /// Bandcamp delivers whatever format the buyer picks at download time, FLAC
    /// included, so a purchase here is lossless by construction.
    pub fn to_offer(&self) -> Option<Offer> {
        if !self.purchasable {
            return None;
        }
        let mut offer = Offer::new(
            Vendor::Bandcamp,
            Acquisition::Purchase,
            Delivery::lossless("FLAC", 16, 44_100),
        )
        .with_note("Choose FLAC, ALAC or WAV at download");
        if let Some((amount, currency)) = &self.price {
            offer = offer.with_price(*amount, currency);
        }
        if let Some(url) = &self.url {
            offer = offer.with_url(url.clone());
        }
        Some(offer)
    }
}

pub fn parse_search(body: &[u8]) -> Outcome<Vec<Hit>> {
    let root = match parse_json(SourceId::Bandcamp, body) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };

    let Some(items) = root.get("results").and_then(Value::as_array) else {
        return Outcome::Stale(Reason::Malformed("no results array".into()));
    };

    let hits = items
        .iter()
        // "a" is an album. Tracks, artists and labels come back in the same
        // list and none of them is an album to rank.
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("a"))
        .filter_map(|item| {
            Some(Hit {
                album_id: item.get("id").and_then(Value::as_u64)?,
                band_id: item.get("band_id").and_then(Value::as_u64)?,
                title: string(item, "name").unwrap_or_default(),
                artist: string(item, "band_name").unwrap_or_default(),
                url: repair_url(&string(item, "url")?),
                art: string(item, "img").filter(|url| !url.is_empty()),
            })
        })
        .collect();

    Outcome::of_collection(hits)
}

pub fn parse_details(body: &[u8], url: Option<String>) -> Outcome<Details> {
    let root = match parse_json(SourceId::Bandcamp, body) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };

    // `title` is the one field that must be there. Its absence means the shape
    // changed, not that the album has no name.
    let Some(title) = string(&root, "title") else {
        return Outcome::Stale(Reason::Malformed("no title in tralbum_details".into()));
    };

    let price = root
        .get("price")
        .and_then(Value::as_f64)
        .filter(|amount| *amount > 0.0)
        .zip(string(&root, "currency"));

    Outcome::Found(Details {
        title,
        // The performer credit, which differs from the page owner on a label
        // page: `band_name` is the label, `tralbum_artist` is the act.
        artist: string(&root, "tralbum_artist"),
        price,
        purchasable: root
            .get("is_purchasable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        downloadable_tracks: root
            .get("num_downloadable_tracks")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        url,
    })
}

/// Pick the hit that is actually the album asked for, or none.
///
/// Bandcamp's search is loose, and for a famous album on a label that is not on
/// Bandcamp it returns nothing but covers: a live search for "Radiohead Kid A"
/// leads with *Radiohead - Kid A* by the Halifax Music Co-op, then a string
/// quartet recital, then two chiptune versions. Every one of those has a title
/// closer to "Kid A" than anything Radiohead have put there.
///
/// So the artist is scored too, and there is a floor. Getting this wrong invents
/// a tier-A offer at the top of the ranking for a record by somebody else, which
/// is the worst answer this application could give.
/// Both halves have to hold independently. A combined average would let a
/// perfect title carry a wrong artist over the line, which is precisely the
/// Halifax Music Co-op case — its page is literally titled "Radiohead - Kid A".
///
/// The cost is a false negative on label pages, where `band_name` is the label
/// rather than the act. That is the right way to be wrong: a missing Bandcamp
/// row says "not checked", while a wrong one says "buy this" about a record
/// nobody asked for.
pub fn best_hit<'a>(hits: &'a [Hit], artist: &str, title: &str) -> Option<&'a Hit> {
    use crate::model::query::fold;
    let ratio = |a: &str, b: &str| rapidfuzz::fuzz::ratio(fold(a).chars(), fold(b).chars());

    hits.iter()
        .filter_map(|hit| {
            let artist_score = ratio(artist, &hit.artist);
            let title_score = ratio(title, &hit.title);
            (artist_score >= ARTIST_FLOOR && title_score >= TITLE_FLOOR)
                .then_some((hit, artist_score + title_score))
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(hit, _)| hit)
}

/// Below these, the hit is somebody else's record.
const ARTIST_FLOOR: f64 = 0.75;
const TITLE_FLOOR: f64 = 0.7;

/// Undo Bandcamp's doubled URLs.
///
/// `fuzzysearch` concatenates the band's subdomain with an already-absolute
/// album URL, so a live response really does contain
/// `https://radiohead.bandcamp.comhttps://radiohead.bandcamp.com/album/kid-a`.
/// Taking the last scheme onwards recovers the real link and leaves a
/// well-formed one untouched.
fn repair_url(url: &str) -> String {
    match url.rfind("https://") {
        Some(index) if index > 0 => url[index..].to_string(),
        _ => url.to_string(),
    }
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
    fn the_doubled_url_bandcamp_really_sends_is_repaired() {
        // Taken verbatim from a live response.
        assert_eq!(
            repair_url(
                "https://radiohead.bandcamp.comhttps://radiohead.bandcamp.com/album/kid-a-mnesia"
            ),
            "https://radiohead.bandcamp.com/album/kid-a-mnesia"
        );
        // A correct URL survives untouched.
        assert_eq!(
            repair_url("https://radiohead.bandcamp.com/album/kid-a"),
            "https://radiohead.bandcamp.com/album/kid-a"
        );
    }

    #[test]
    fn a_search_response_yields_albums_with_ids_art_and_repaired_urls() {
        let body = br#"{"results":[
          {"type":"a","id":3317386587,"band_id":3957198221,"name":"KID A MNESIA",
           "band_name":"Radiohead","img":"https://f4.bcbits.com/img/3185643660_3.jpg",
           "url":"https://radiohead.bandcamp.comhttps://radiohead.bandcamp.com/album/kid-a-mnesia"},
          {"type":"b","id":1,"band_id":2,"name":"Radiohead","band_name":"Radiohead","url":"https://radiohead.bandcamp.com"},
          {"type":"t","id":3,"band_id":2,"name":"Everything In Its Right Place","band_name":"Radiohead","url":"https://x.bandcamp.com/track/y"}]}"#;

        let Outcome::Found(hits) = parse_search(body) else {
            panic!("expected hits");
        };
        // Only the album; the band and the track are not albums to rank.
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].album_id, 3317386587);
        assert_eq!(hits[0].band_id, 3957198221);
        assert_eq!(
            hits[0].url,
            "https://radiohead.bandcamp.com/album/kid-a-mnesia"
        );
        assert!(hits[0].art.is_some());
    }

    #[test]
    fn an_album_on_bandcamp_that_is_not_for_sale_produces_no_offer() {
        // The live response for Kid A: a real price, and `is_purchasable: false`.
        // Reading the price and skipping the flag would invent the top-ranked
        // offer in the list out of nothing.
        let body = br#"{"title":"Kid A","tralbum_artist":"Radiohead","currency":"USD",
            "price":9.99,"is_purchasable":false,"free_download":false,
            "num_downloadable_tracks":11,"type":"a"}"#;
        let Outcome::Found(details) = parse_details(body, None) else {
            panic!("expected details");
        };
        assert_eq!(details.price, Some((9.99, "USD".into())));
        assert!(!details.purchasable);
        assert_eq!(details.to_offer(), None);
    }

    #[test]
    fn a_purchasable_album_is_a_lossless_tier_a_offer() {
        let body = br#"{"title":"In Rainbows","tralbum_artist":"Radiohead","currency":"GBP",
            "price":8.0,"is_purchasable":true,"num_downloadable_tracks":10,"type":"a"}"#;
        let Outcome::Found(details) = parse_details(body, Some("https://x/album/y".into())) else {
            panic!("expected details");
        };
        let offer = details.to_offer().expect("an offer");
        assert_eq!(tier_of(&offer), Tier::A);
        assert!(offer.delivery.lossless);
        assert_eq!(
            offer.price,
            Some(crate::model::offer::Price {
                amount: 8.0,
                currency: "GBP".into()
            })
        );
        assert_eq!(offer.url.as_deref(), Some("https://x/album/y"));
    }

    #[test]
    fn a_label_page_keeps_the_performer_separate_from_the_page_owner() {
        // On a label's page `band_name` is the label and `tralbum_artist` is the
        // act. Showing the label as the artist makes the match look wrong.
        let body = br#"{"title":"Combined Minds","tralbum_artist":"Mortaja",
            "is_purchasable":true,"price":7.0,"currency":"EUR"}"#;
        let Outcome::Found(details) = parse_details(body, None) else {
            panic!("expected details");
        };
        assert_eq!(details.artist.as_deref(), Some("Mortaja"));
    }

    #[test]
    fn a_changed_shape_is_stale_rather_than_empty() {
        // The whole reason the fourth variant exists. If this returned Empty,
        // a broken Bandcamp reader would look exactly like an album Bandcamp
        // does not stock, and the best offer would vanish from every ranking
        // without a word.
        assert!(matches!(
            parse_search(br#"{"items":[]}"#),
            Outcome::Stale(_)
        ));
        assert!(matches!(
            parse_details(br#"{"error":"gone"}"#, None),
            Outcome::Stale(_)
        ));
        assert!(matches!(
            parse_search(b"<html>Just a moment...</html>"),
            Outcome::Stale(_)
        ));
        // Whereas a genuine no-results answer is Empty.
        assert_eq!(parse_search(br#"{"results":[]}"#), Outcome::Empty);
    }

    #[test]
    fn a_missing_purchasable_flag_is_read_as_not_for_sale() {
        // Fail closed. Inventing a purchase is worse than missing one.
        let Outcome::Found(details) =
            parse_details(br#"{"title":"X","price":5.0,"currency":"USD"}"#, None)
        else {
            panic!("expected details");
        };
        assert!(!details.purchasable);
        assert_eq!(details.to_offer(), None);
    }
}
