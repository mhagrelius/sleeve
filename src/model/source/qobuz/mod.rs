//! Qobuz: a shop and a streaming service behind one API.
//!
//! The reason this source is worth the trouble of an unofficial `app_id` is that
//! Qobuz album objects say which of the two you are looking at. `purchasable`
//! means the store will sell you the files; `streamable` means a subscription
//! plays them; `hires` means the files are above CD. One album can be all three,
//! and it is then **two offers, in two different tiers** — a tier-A download and
//! a tier-C stream — from a single response.
//!
//! Collapsing those into one row is the easiest way to get this whole model
//! wrong, and it is why [`Album::to_offers`] returns a vector.
//!
//! **This source needs a Qobuz account.** The catalogue endpoints answer `401
//! User authentication is required` to an `app_id` alone — verified against the
//! live API, and the refusal is recorded as a fixture. They want an
//! `X-User-Auth-Token` from a logged-in `play.qobuz.com` session as well. That
//! is a narrower footprint than it sounds: the token reads the catalogue the way
//! the website does, and the `app_secret` and request signing that the download
//! endpoints need are never involved, because Sleeve never asks for a file URL.
//! Without a token the source reports itself unconfigured and is skipped.

pub mod credentials;

use serde_json::Value;

use super::{parse_json, Outcome, Reason, Request, SourceId};
use crate::model::offer::{Acquisition, Delivery, Friction, Offer, Vendor};

const BASE: &str = "https://www.qobuz.com/api.json/0.2";

/// Search the catalogue for albums.
pub fn search_albums(query: &str, app_id: &str, user_token: &str, locale: &str) -> Request {
    Request::get(
        SourceId::QobuzStore,
        format!(
            "{BASE}/album/search?query={}&limit=25&app_id={}",
            encode(query),
            encode(app_id)
        ),
    )
    .header("Accept", "application/json")
    .header("X-App-Id", app_id.to_string())
    .header("X-User-Auth-Token", user_token.to_string())
    // Qobuz varies price and availability by storefront, so the locale is not
    // decoration — it is the difference between a price you can pay and one you
    // cannot.
    .header("Accept-Language", locale.to_string())
}

/// One album as Qobuz describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Album {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub year: Option<i32>,
    pub track_count: usize,
    pub purchasable: bool,
    pub streamable: bool,
    pub hires: bool,
    pub bit_depth: Option<u8>,
    pub sample_rate_hz: Option<u32>,
    pub price: Option<(f64, String)>,
    pub url: Option<String>,
    pub cover: Option<String>,
    /// Set when Qobuz says the release is not available in this storefront.
    pub region_locked: bool,
}

impl Album {
    /// Every offer this one album represents — up to two, in two tiers.
    pub fn to_offers(&self) -> Vec<Offer> {
        let mut offers = Vec::new();
        let delivery = || {
            Delivery::lossless(
                "FLAC",
                self.bit_depth.unwrap_or(16),
                self.sample_rate_hz.unwrap_or(44_100),
            )
        };

        if self.purchasable {
            let mut offer = Offer::new(Vendor::QobuzStore, Acquisition::Purchase, delivery());
            if let Some((amount, currency)) = &self.price {
                offer = offer.with_price(*amount, currency);
            }
            if let Some(url) = &self.url {
                offer = offer.with_url(url.clone());
            }
            if self.region_locked {
                offer = offer.with_friction(Friction::RegionLocked);
            }
            offers.push(offer);
        }

        if self.streamable {
            let mut offer = Offer::new(Vendor::Qobuz, Acquisition::Subscription, delivery());
            if let Some(url) = &self.url {
                offer = offer.with_url(url.clone());
            }
            if self.region_locked {
                offer = offer.with_friction(Friction::RegionLocked);
            }
            offers.push(offer);
        }

        offers
    }
}

pub fn parse_search(body: &[u8]) -> Outcome<Vec<Album>> {
    let root = match parse_json(SourceId::QobuzStore, body) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };

    // A rotated app_id shows up as a 401 body rather than a transport error, and
    // it is fixable by refreshing the credential — so it is reported as a
    // configuration problem, not a parse failure.
    if let Some(status) = root.get("status").and_then(Value::as_str) {
        if status.eq_ignore_ascii_case("error") {
            let message = root
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("rejected");
            return Outcome::Unusable(Reason::NotConfigured(format!("Qobuz: {message}")));
        }
    }

    let Some(items) = root
        .get("albums")
        .and_then(|albums| albums.get("items"))
        .and_then(Value::as_array)
    else {
        return Outcome::Stale(Reason::Malformed("no albums.items array".into()));
    };

    Outcome::of_collection(items.iter().filter_map(parse_album).collect())
}

pub fn parse_album(item: &Value) -> Option<Album> {
    let id = item.get("id").and_then(|id| {
        id.as_str()
            .map(str::to_string)
            .or_else(|| id.as_u64().map(|n| n.to_string()))
    })?;

    Some(Album {
        id,
        title: string(item, "title").unwrap_or_default(),
        artist: item
            .get("artist")
            .and_then(|artist| string(artist, "name"))
            .unwrap_or_default(),
        year: string(item, "release_date_original")
            .as_deref()
            .and_then(|date| date.get(..4).and_then(|year| year.parse().ok())),
        track_count: item
            .get("tracks_count")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize,
        purchasable: item
            .get("purchasable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        streamable: item
            .get("streamable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        hires: item.get("hires").and_then(Value::as_bool).unwrap_or(false),
        bit_depth: item
            .get("maximum_bit_depth")
            .and_then(Value::as_u64)
            .map(|depth| depth as u8),
        sample_rate_hz: item
            .get("maximum_sampling_rate")
            .and_then(Value::as_f64)
            // Qobuz reports kHz as a float: 44.1, 96, 192.
            .map(|khz| (khz * 1000.0).round() as u32),
        price: item
            .get("price")
            .and_then(Value::as_f64)
            .filter(|amount| *amount > 0.0)
            .zip(string(item, "currency")),
        url: string(item, "url"),
        cover: item
            .get("image")
            .and_then(|image| string(image, "large").or_else(|| string(image, "small"))),
        // Both flags false with a release present means the storefront has it
        // listed but will not serve this locale.
        region_locked: !item
            .get("purchasable")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            && !item
                .get("streamable")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            && item.get("purchasable_at").is_some(),
    })
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

    const BODY: &[u8] = br#"{"albums":{"items":[{
        "id":"0060254720route","title":"Kid A","tracks_count":10,
        "artist":{"name":"Radiohead"},
        "release_date_original":"2000-10-02",
        "purchasable":true,"streamable":true,"hires":true,
        "maximum_bit_depth":24,"maximum_sampling_rate":96.0,
        "price":13.49,"currency":"GBP",
        "url":"https://www.qobuz.com/gb-en/album/kid-a/0060254720",
        "image":{"large":"https://static.qobuz.com/images/covers/x_600.jpg"}}]}}"#;

    #[test]
    fn one_album_that_is_both_sold_and_streamed_becomes_two_offers_in_two_tiers() {
        // The distinction this source exists for. A single row here would put
        // either a purchase in the streaming tier or a stream in the purchase
        // tier, and both are the failure the whole model is built to avoid.
        let Outcome::Found(albums) = parse_search(BODY) else {
            panic!("expected albums");
        };
        let offers = albums[0].to_offers();
        assert_eq!(offers.len(), 2);

        let store = offers
            .iter()
            .find(|o| o.vendor == Vendor::QobuzStore)
            .unwrap();
        let stream = offers.iter().find(|o| o.vendor == Vendor::Qobuz).unwrap();
        assert_eq!(tier_of(store), Tier::A);
        assert_eq!(tier_of(stream), Tier::C);
        // Only the purchase carries a price; the stream is part of a subscription.
        assert!(store.price.is_some());
        assert!(stream.price.is_none());
    }

    #[test]
    fn the_sample_rate_is_read_as_kilohertz_and_stored_as_hertz() {
        let Outcome::Found(albums) = parse_search(BODY) else {
            panic!("expected albums");
        };
        assert_eq!(albums[0].sample_rate_hz, Some(96_000));
        assert_eq!(albums[0].bit_depth, Some(24));
    }

    #[test]
    fn a_stream_only_album_produces_no_purchase_offer() {
        let body = br#"{"albums":{"items":[{"id":"1","title":"X","artist":{"name":"Y"},
            "purchasable":false,"streamable":true,"maximum_bit_depth":16,
            "maximum_sampling_rate":44.1}]}}"#;
        let Outcome::Found(albums) = parse_search(body) else {
            panic!("expected albums");
        };
        let offers = albums[0].to_offers();
        assert_eq!(offers.len(), 1);
        assert_eq!(offers[0].vendor, Vendor::Qobuz);
        assert_eq!(albums[0].sample_rate_hz, Some(44_100));
    }

    #[test]
    fn an_album_listed_but_served_to_no_one_here_is_region_locked() {
        let body = br#"{"albums":{"items":[{"id":"1","title":"X","artist":{"name":"Y"},
            "purchasable":false,"streamable":false,"purchasable_at":1600000000}]}}"#;
        let Outcome::Found(albums) = parse_search(body) else {
            panic!("expected albums");
        };
        assert!(albums[0].region_locked);
    }

    #[test]
    fn a_rejected_app_id_is_a_configuration_problem() {
        // Qobuz answers a rotated id with an error body, and the fix is to
        // refresh the credential — not to look at the parser.
        let body = br#"{"status":"error","code":401,"message":"Invalid app_id"}"#;
        assert!(matches!(
            parse_search(body),
            Outcome::Unusable(Reason::NotConfigured(_))
        ));
    }

    #[test]
    fn no_results_is_empty_and_a_changed_shape_is_stale() {
        assert_eq!(parse_search(br#"{"albums":{"items":[]}}"#), Outcome::Empty);
        assert!(matches!(
            parse_search(br#"{"tracks":{}}"#),
            Outcome::Stale(_)
        ));
    }

    #[test]
    fn the_search_request_carries_the_app_id_and_token_but_never_a_secret() {
        // The secret and the request signature exist to mint file URLs. Sleeve
        // has no business holding either, and this is the assertion that keeps
        // it that way.
        let request = search_albums("kid a", "798273057", "usertoken", "en-GB");
        assert!(request.url.contains("app_id=798273057"));
        assert!(request
            .headers
            .iter()
            .any(|(name, value)| name == "X-User-Auth-Token" && value == "usertoken"));
        assert!(!request.url.contains("secret"));
        assert!(!request.url.contains("signature"));
        assert!(!request.url.contains("getFileUrl"));
    }

    #[test]
    fn the_live_refusal_to_an_unauthenticated_call_reads_as_unconfigured() {
        // Recorded from the real API: an app_id with no user token gets a 401
        // with this body. It has to route the person to their config file rather
        // than look like Qobuz changed shape.
        let body = br#"{"message":"User authentication is required. (Root=1-6a6f662d)","status":"error","code":401}"#;
        let Outcome::Unusable(Reason::NotConfigured(message)) = parse_search(body) else {
            panic!("expected an unconfigured outcome");
        };
        assert!(message.contains("authentication"));
    }
}
