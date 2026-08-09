//! Discogs: physical pressings and what they go for.
//!
//! The only source that knows about objects. It supplies the tier-A physical
//! options — a new or used CD is a lossless purchase once ripped — and its
//! master/release split lines up neatly with MusicBrainz's release-group/release
//! split, which is what lets a specific pressing be priced rather than an album
//! in the abstract.
//!
//! **No token required.** Both the search and release endpoints answer
//! unauthenticated — checked against the live API rather than taken from the
//! brief, which said otherwise. A token raises the rate limit from 25 requests a
//! minute to 60 and nothing else, so it is an optimisation rather than a
//! prerequisite, and [`token`] simply omits the header when there is none.

use serde_json::Value;

use super::{parse_json, Outcome, Reason, Request, SourceId};
use crate::model::offer::{Acquisition, Delivery, Friction, Offer, Vendor};

const BASE: &str = "https://api.discogs.com";

/// Search masters — the album as a work, not one pressing of it.
///
/// Only used when MusicBrainz has no `discogs` relation for the album. An
/// editor-made link is an exact match; a text search can land on a tribute
/// record, so it is the fallback rather than the first choice.
pub fn search_masters(artist: &str, album: &str, token: &str, user_agent: &str) -> Request {
    authorise(
        Request::get(
            SourceId::Discogs,
            format!(
                "{BASE}/database/search?artist={}&release_title={}&type=master&per_page=25",
                encode(artist),
                encode(album)
            ),
        ),
        token,
    )
    .header("User-Agent", user_agent)
}

/// The pressings of one master, newest listing first.
pub fn master_versions(id: u64, token: &str, user_agent: &str) -> Request {
    authorise(
        Request::get(
            SourceId::Discogs,
            format!("{BASE}/masters/{id}/versions?per_page=25"),
        ),
        token,
    )
    .header("User-Agent", user_agent)
}

/// Attach a token if there is one.
///
/// Discogs answers either way; the header only buys a higher rate limit. Sending
/// `Discogs token=` with nothing after it is worse than sending nothing, so an
/// empty string means no header.
fn authorise(request: Request, token: &str) -> Request {
    if token.trim().is_empty() {
        request
    } else {
        request.header("Authorization", format!("Discogs token={}", token.trim()))
    }
}

/// One pressing from a master's version list.
#[derive(Debug, Clone, PartialEq)]
pub struct Version {
    pub id: u64,
    pub year: Option<i32>,
    pub country: Option<String>,
    pub format: Option<String>,
}

pub fn parse_versions(body: &[u8]) -> Outcome<Vec<Version>> {
    let root = match parse_json(SourceId::Discogs, body) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };
    let Some(items) = root.get("versions").and_then(Value::as_array) else {
        return Outcome::Stale(Reason::Malformed("no versions array".into()));
    };
    Outcome::of_collection(
        items
            .iter()
            .filter_map(|item| {
                Some(Version {
                    id: item.get("id").and_then(Value::as_u64)?,
                    year: item.get("released").and_then(as_year),
                    country: string(item, "country"),
                    format: string(item, "format"),
                })
            })
            .collect(),
    )
}

/// One pressing, with its marketplace statistics.
///
/// `curr_abbr` is what makes the price come back in the configured currency
/// rather than in dollars; a converted price shown as a local one is a lie about
/// what the checkout will charge.
pub fn release(id: u64, currency: &str, token: &str, user_agent: &str) -> Request {
    authorise(
        Request::get(
            SourceId::Discogs,
            format!("{BASE}/releases/{id}?curr_abbr={}", encode(currency)),
        ),
        token,
    )
    .header("User-Agent", user_agent)
}

/// A master release in the Discogs database.
#[derive(Debug, Clone, PartialEq)]
pub struct Master {
    pub id: u64,
    pub title: String,
    pub year: Option<i32>,
    pub url: Option<String>,
    pub thumbnail: Option<String>,
    pub formats: Vec<String>,
}

/// What the marketplace says about one pressing.
#[derive(Debug, Clone, PartialEq)]
pub struct Listing {
    pub id: u64,
    pub title: String,
    pub year: Option<i32>,
    pub url: Option<String>,
    pub formats: Vec<String>,
    pub for_sale: u64,
    pub lowest_price: Option<(f64, String)>,
    /// Set when the pressing is a boxset or multi-disc compilation rather than
    /// the album on its own.
    pub boxset: bool,
}

impl Listing {
    /// A used disc from the marketplace.
    ///
    /// Always [`Vendor::PhysicalUsed`], because Discogs' marketplace is
    /// second-hand: the seller keeps the money and the artist sees none of it.
    /// That is a fact about the transaction, and the payout table scores it at
    /// zero for exactly that reason — the disc is still tier A, because you end
    /// up owning lossless files, but it earns no payout bonus at all.
    pub fn to_offer(&self) -> Option<Offer> {
        if self.for_sale == 0 {
            return None;
        }
        let mut offer = Offer::new(
            Vendor::PhysicalUsed,
            Acquisition::Purchase,
            Delivery::lossless("CD", 16, 44_100),
        )
        .with_friction(Friction::RequiresRipping);

        if let Some((amount, currency)) = &self.lowest_price {
            offer = offer.with_price(*amount, currency);
        }
        if let Some(url) = &self.url {
            offer = offer.with_url(url.clone());
        }
        if self.boxset {
            offer = offer.with_friction(Friction::BoxsetOnly);
        }
        if let Some(year) = self.year {
            offer = offer.with_edition(format!("{year} pressing"));
        }
        Some(offer.with_note(format!("{} for sale on Discogs", self.for_sale)))
    }
}

pub fn parse_masters(body: &[u8]) -> Outcome<Vec<Master>> {
    let root = match parse_json(SourceId::Discogs, body) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };

    // Discogs answers an expired or missing token with a JSON body carrying a
    // message, not with a results array. That is a configuration problem the
    // person can fix, so it is reported as one rather than as a broken parser.
    if let Some(message) = root.get("message").and_then(Value::as_str) {
        return Outcome::Unusable(Reason::NotConfigured(message.to_string()));
    }

    let Some(items) = root.get("results").and_then(Value::as_array) else {
        return Outcome::Stale(Reason::Malformed("no results array".into()));
    };

    let masters = items
        .iter()
        .filter_map(|item| {
            Some(Master {
                id: item.get("id").and_then(Value::as_u64)?,
                title: string(item, "title").unwrap_or_default(),
                year: item.get("year").and_then(as_year),
                url: string(item, "uri").map(|path| format!("https://www.discogs.com{path}")),
                thumbnail: string(item, "cover_image").or_else(|| string(item, "thumb")),
                formats: strings(item, "format"),
            })
        })
        .collect();

    Outcome::of_collection(masters)
}

pub fn parse_release(body: &[u8]) -> Outcome<Listing> {
    let root = match parse_json(SourceId::Discogs, body) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };
    if let Some(message) = root.get("message").and_then(Value::as_str) {
        return Outcome::Unusable(Reason::NotConfigured(message.to_string()));
    }
    let Some(id) = root.get("id").and_then(Value::as_u64) else {
        return Outcome::Stale(Reason::Malformed("no release id".into()));
    };

    let formats: Vec<String> = root
        .get("formats")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| string(entry, "name"))
                .collect()
        })
        .unwrap_or_default();

    // A boxset is flagged in the free-text descriptions rather than by a field.
    let boxset = root
        .get("formats")
        .and_then(Value::as_array)
        .map(|entries| {
            entries.iter().any(|entry| {
                strings(entry, "descriptions")
                    .iter()
                    .any(|description| description.eq_ignore_ascii_case("Box Set"))
            })
        })
        .unwrap_or(false);

    let lowest_price = root
        .get("lowest_price")
        .and_then(Value::as_f64)
        .filter(|amount| *amount > 0.0)
        .map(|amount| {
            (
                amount,
                // The API echoes the requested currency in the price, not in a
                // field, so the caller's choice is the answer.
                string(&root, "currency").unwrap_or_else(|| "USD".to_string()),
            )
        });

    Outcome::Found(Listing {
        id,
        title: string(&root, "title").unwrap_or_default(),
        year: root.get("year").and_then(as_year),
        url: string(&root, "uri"),
        formats,
        for_sale: root
            .get("num_for_sale")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        lowest_price,
        boxset,
    })
}

fn as_year(value: &Value) -> Option<i32> {
    value
        .as_i64()
        .map(|year| year as i32)
        .or_else(|| value.as_str().and_then(|text| text.get(..4)?.parse().ok()))
        .filter(|year| *year > 1000)
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
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
    use crate::model::weights::Weights;

    #[test]
    fn a_bad_token_is_a_configuration_problem_not_a_broken_parser() {
        // Reached only when someone sets a *wrong* token, since no token at all
        // works fine. It must not be reported as "Discogs changed shape", which
        // would send them looking at the wrong thing.
        let body = br#"{"message":"You must authenticate to access this resource."}"#;
        assert!(matches!(
            parse_masters(body),
            Outcome::Unusable(Reason::NotConfigured(_))
        ));
    }

    #[test]
    fn no_token_means_no_authorization_header_rather_than_an_empty_one() {
        // Discogs answers unauthenticated at 25 requests a minute. Sending
        // `Discogs token=` with nothing after it gets a 401 instead.
        for request in [
            search_masters("Radiohead", "Kid A", "", "Sleeve/0.1"),
            release(7, "GBP", "   ", "Sleeve/0.1"),
            master_versions(21501, "", "Sleeve/0.1"),
        ] {
            assert!(
                !request
                    .headers
                    .iter()
                    .any(|(name, _)| name == "Authorization"),
                "sent an empty Authorization header"
            );
            assert!(request.headers.iter().any(|(name, _)| name == "User-Agent"));
        }
    }

    #[test]
    fn a_used_cd_is_tier_a_and_earns_no_payout_bonus() {
        // Both halves matter. Owning lossless files puts it in tier A whatever
        // the seller is, and resale paying the artist nothing is what keeps it
        // below every shop in that tier.
        let listing = Listing {
            id: 1,
            title: "Radiohead - Kid A".into(),
            year: Some(2000),
            url: Some("https://www.discogs.com/release/1".into()),
            formats: vec!["CD".into()],
            for_sale: 42,
            lowest_price: Some((4.50, "GBP".into())),
            boxset: false,
        };
        let offer = listing.to_offer().expect("an offer");
        assert_eq!(tier_of(&offer), Tier::A);
        assert_eq!(Weights::default().payout_for(offer.vendor), 0);
        assert!(offer.frictions.contains(&Friction::RequiresRipping));
        assert_eq!(offer.edition.as_deref(), Some("2000 pressing"));
    }

    #[test]
    fn a_pressing_with_no_copies_for_sale_is_not_an_offer() {
        // "Exists in the database" is not "you can buy it". Principle four.
        let listing = Listing {
            id: 1,
            title: "x".into(),
            year: None,
            url: None,
            formats: vec!["CD".into()],
            for_sale: 0,
            lowest_price: None,
            boxset: false,
        };
        assert_eq!(listing.to_offer(), None);
    }

    #[test]
    fn a_boxset_only_pressing_carries_the_boxset_friction() {
        let body = br#"{"id":7,"title":"The Complete Works","year":2011,
            "uri":"https://www.discogs.com/release/7","num_for_sale":3,
            "lowest_price":89.0,"currency":"GBP",
            "formats":[{"name":"CD","descriptions":["Album","Box Set"]}]}"#;
        let Outcome::Found(listing) = parse_release(body) else {
            panic!("expected a listing");
        };
        assert!(listing.boxset);
        let offer = listing.to_offer().unwrap();
        assert!(offer.frictions.contains(&Friction::BoxsetOnly));
    }

    #[test]
    fn a_zero_lowest_price_is_no_price_rather_than_free() {
        let body = br#"{"id":7,"num_for_sale":1,"lowest_price":0,"formats":[{"name":"CD"}]}"#;
        let Outcome::Found(listing) = parse_release(body) else {
            panic!("expected a listing");
        };
        assert_eq!(listing.lowest_price, None);
    }

    #[test]
    fn a_year_parses_whether_discogs_sends_a_number_or_a_string() {
        for body in [
            br#"{"results":[{"id":1,"title":"a","year":2000}]}"#.as_slice(),
            br#"{"results":[{"id":1,"title":"a","year":"2000"}]}"#.as_slice(),
        ] {
            let Outcome::Found(masters) = parse_masters(body) else {
                panic!("expected masters");
            };
            assert_eq!(masters[0].year, Some(2000));
        }
        // And a nonsense year is dropped rather than shown.
        let Outcome::Found(masters) = parse_masters(br#"{"results":[{"id":1,"year":0}]}"#) else {
            panic!("expected masters");
        };
        assert_eq!(masters[0].year, None);
    }

    #[test]
    fn a_master_version_list_parses_into_pressings() {
        let body = br#"{"versions":[
            {"id":1854456,"released":"2000","country":"UK","format":"CD, Album"},
            {"id":9,"released":"","country":"US","format":"2\u00d7Vinyl"}]}"#;
        let Outcome::Found(versions) = parse_versions(body) else {
            panic!("expected versions");
        };
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].id, 1854456);
        assert_eq!(versions[0].year, Some(2000));
        assert_eq!(versions[1].year, None);
    }

    #[test]
    fn the_request_carries_the_token_the_agent_and_the_currency() {
        let request = release(7, "GBP", "secret", "Sleeve/0.1");
        assert!(request.url.contains("curr_abbr=GBP"));
        assert!(request
            .headers
            .iter()
            .any(|(name, value)| name == "Authorization" && value.contains("secret")));
        // Discogs rejects requests with no User-Agent outright.
        assert!(request.headers.iter().any(|(name, _)| name == "User-Agent"));
    }
}
