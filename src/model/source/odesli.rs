//! Odesli (song.link): one call, every streaming service.
//!
//! Free, no key, and it answers the question "who has this" for Spotify, Apple
//! Music, Tidal, Deezer, Amazon and YouTube in a single request — which is worth
//! a great deal, because the alternative is six sources with six auth stories.
//!
//! What it does *not* tell you is quality. It reports that a platform carries
//! the album, not what bitrate it hands you, so the delivery for each vendor is
//! decided here from what that service offers its subscribers. Spotify is the
//! only one that is a matter of configuration rather than fact — see
//! [`Catalogue::spotify_lossless`].
//!
//! Its unauthenticated budget is roughly ten requests a minute, the tightest of
//! any source. It is therefore only ever called once a specific release has been
//! chosen, never while typing in the search box.

use serde_json::Value;

use super::{parse_json, Outcome, Reason, Request, SourceId};
use crate::model::offer::{Acquisition, Delivery, Offer, Vendor};

/// Look up every platform carrying an album, seeded from its iTunes id.
///
/// Seeding from iTunes rather than from a URL because the iTunes Search API is
/// already being called and hands back a `collectionId` for free, which makes
/// this one extra request rather than two.
pub fn links_for_itunes_album(collection_id: u64, country: &str) -> Request {
    Request::get(
        SourceId::Odesli,
        format!(
            "https://api.song.link/v1-alpha.1/links?platform=itunes&type=album&id={collection_id}&userCountry={country}"
        ),
    )
    .header("Accept", "application/json")
}

/// Which services carry the album, and where.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Catalogue {
    pub links: Vec<(Vendor, String)>,
    /// Whether Spotify should be treated as lossless.
    ///
    /// Spotify Premium began serving 24-bit/44.1 kHz FLAC in September 2025 and
    /// reached 50-plus markets by that October, so by the tier table's own
    /// criterion — tier is lossless versus lossy, payout is a separate modifier —
    /// Spotify is tier C in those markets and tier D outside them. It is
    /// configuration rather than a constant because it depends on the locale and
    /// on having Premium, and getting it wrong in either direction misreports
    /// what a person can actually hear. Its payout modifier stays at zero either
    /// way, so it still sorts below Qobuz and Tidal within the tier.
    pub spotify_lossless: bool,
}

impl Catalogue {
    pub fn to_offers(&self) -> Vec<Offer> {
        self.links
            .iter()
            .map(|(vendor, url)| {
                Offer::new(
                    *vendor,
                    Acquisition::Subscription,
                    delivery_for(*vendor, self.spotify_lossless),
                )
                .with_url(url.clone())
            })
            .collect()
    }
}

/// What a subscription to each service actually delivers.
fn delivery_for(vendor: Vendor, spotify_lossless: bool) -> Delivery {
    match vendor {
        // Hi-res FLAC to subscribers.
        Vendor::Qobuz | Vendor::Tidal | Vendor::AppleMusic | Vendor::AmazonMusicHd => {
            Delivery::lossless("FLAC", 24, 96_000)
        }
        // Lossless, but CD quality rather than hi-res.
        Vendor::Deezer => Delivery::lossless("FLAC", 16, 44_100),
        Vendor::Spotify => {
            if spotify_lossless {
                Delivery::lossless("FLAC", 24, 44_100)
            } else {
                Delivery::lossy("Ogg Vorbis 320")
            }
        }
        Vendor::YouTubeMusic => Delivery::lossy("Opus 256"),
        // Not a streaming service; nothing here should produce one, and if it
        // does, lossy is the answer that cannot promote it above a purchase.
        _ => Delivery::lossy("Unknown"),
    }
}

/// Map Odesli's platform keys onto vendors.
///
/// Only subscription services are taken. Odesli also returns purchase links for
/// iTunes and Amazon's MP3 store, but the iTunes Search API answers that
/// question with a price attached, and a link with no price is not an offer this
/// application can rank.
fn vendor_for(platform: &str) -> Option<Vendor> {
    match platform {
        "spotify" => Some(Vendor::Spotify),
        "appleMusic" => Some(Vendor::AppleMusic),
        "tidal" => Some(Vendor::Tidal),
        "deezer" => Some(Vendor::Deezer),
        "amazonMusic" => Some(Vendor::AmazonMusicHd),
        "youtubeMusic" => Some(Vendor::YouTubeMusic),
        _ => None,
    }
}

pub fn parse_catalogue(body: &[u8], spotify_lossless: bool) -> Outcome<Catalogue> {
    let root = match parse_json(SourceId::Odesli, body) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };

    let Some(platforms) = root.get("linksByPlatform").and_then(Value::as_object) else {
        return Outcome::Stale(Reason::Malformed("no linksByPlatform object".into()));
    };

    let mut links: Vec<(Vendor, String)> = platforms
        .iter()
        .filter_map(|(platform, entry)| {
            let vendor = vendor_for(platform)?;
            let url = entry.get("url").and_then(Value::as_str)?;
            Some((vendor, url.to_string()))
        })
        .collect();
    links.sort_by_key(|(vendor, _)| *vendor);

    if links.is_empty() {
        return Outcome::Empty;
    }
    Outcome::Found(Catalogue {
        links,
        spotify_lossless,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tier::{tier_of, Tier};

    const BODY: &[u8] = br#"{"entityUniqueId":"itunes::1","userCountry":"GB",
        "linksByPlatform":{
          "spotify":{"url":"https://open.spotify.com/album/s"},
          "appleMusic":{"url":"https://music.apple.com/gb/album/a"},
          "tidal":{"url":"https://listen.tidal.com/album/t"},
          "deezer":{"url":"https://www.deezer.com/album/d"},
          "amazonMusic":{"url":"https://music.amazon.com/albums/z"},
          "youtubeMusic":{"url":"https://music.youtube.com/playlist?list=y"},
          "itunes":{"url":"https://music.apple.com/gb/album/i"},
          "pandora":{"url":"https://pandora.com/p"}}}"#;

    #[test]
    fn every_subscription_platform_becomes_an_offer() {
        let Outcome::Found(catalogue) = parse_catalogue(BODY, false) else {
            panic!("expected a catalogue");
        };
        let vendors: Vec<Vendor> = catalogue.links.iter().map(|(v, _)| *v).collect();
        assert!(vendors.contains(&Vendor::Spotify));
        assert!(vendors.contains(&Vendor::Tidal));
        assert!(vendors.contains(&Vendor::AmazonMusicHd));
        assert_eq!(catalogue.to_offers().len(), 6);
    }

    #[test]
    fn purchase_links_and_unknown_platforms_are_left_alone() {
        // The iTunes link has no price on it. Ranking a purchase with no price
        // against ones that have prices would be a worse answer than omitting it,
        // and the iTunes source supplies the same album properly.
        let Outcome::Found(catalogue) = parse_catalogue(BODY, false) else {
            panic!("expected a catalogue");
        };
        assert_eq!(catalogue.links.len(), 6);
        assert!(catalogue
            .to_offers()
            .iter()
            .all(|offer| offer.acquisition == Acquisition::Subscription));
    }

    #[test]
    fn spotify_moves_between_tiers_with_the_configured_locale() {
        let lossy = parse_catalogue(BODY, false).found().unwrap();
        let lossless = parse_catalogue(BODY, true).found().unwrap();

        let spotify = |catalogue: &Catalogue| {
            catalogue
                .to_offers()
                .into_iter()
                .find(|offer| offer.vendor == Vendor::Spotify)
                .expect("a spotify offer")
        };
        assert_eq!(tier_of(&spotify(&lossy)), Tier::D);
        assert_eq!(tier_of(&spotify(&lossless)), Tier::C);
    }

    #[test]
    fn a_lossless_spotify_still_sorts_below_qobuz_and_tidal() {
        // Moving Spotify up a tier must not disturb the payout ordering inside
        // that tier — that is the whole reason tier and payout are separate.
        use crate::model::score::rank;
        use crate::model::weights::Weights;

        let catalogue = parse_catalogue(BODY, true).found().unwrap();
        let mut offers = catalogue.to_offers();
        offers.push(
            Offer::new(
                Vendor::Qobuz,
                Acquisition::Subscription,
                Delivery::lossless("FLAC", 24, 96_000),
            )
            .with_url("https://qobuz.com/x"),
        );
        let ranked = rank(&offers, &Weights::default());
        let order: Vec<Vendor> = ranked.ranked.iter().map(|s| s.offer.vendor).collect();
        let position = |vendor: Vendor| order.iter().position(|v| *v == vendor).unwrap();
        assert!(position(Vendor::Qobuz) < position(Vendor::Spotify));
        assert!(position(Vendor::Tidal) < position(Vendor::Spotify));
    }

    #[test]
    fn no_platforms_is_empty_and_a_changed_shape_is_stale() {
        assert_eq!(
            parse_catalogue(br#"{"linksByPlatform":{}}"#, false),
            Outcome::Empty
        );
        assert!(matches!(
            parse_catalogue(br#"{"statusCode":429}"#, false),
            Outcome::Stale(_)
        ));
    }

    #[test]
    fn the_request_carries_the_locale_so_the_answer_is_about_our_region() {
        let request = links_for_itunes_album(1109714933, "GB");
        assert!(request.url.contains("id=1109714933"));
        assert!(request.url.contains("userCountry=GB"));
    }

    #[test]
    fn no_streaming_offer_can_ever_reach_a_purchase_tier() {
        let Outcome::Found(catalogue) = parse_catalogue(BODY, true) else {
            panic!("expected a catalogue");
        };
        for offer in catalogue.to_offers() {
            assert!(
                tier_of(&offer) >= Tier::C,
                "{} escaped to a purchase tier",
                offer.vendor
            );
        }
    }
}
