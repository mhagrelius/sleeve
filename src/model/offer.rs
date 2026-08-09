//! What one vendor will actually sell or stream you.
//!
//! An [`Offer`] is a claim about a specific album at a specific vendor: how it
//! is delivered, at what price, in what edition, and what stands between you and
//! it. It carries no score. Scoring is [`super::score`]'s job, and keeping the
//! two apart is what lets a weight change be tested without rebuilding a single
//! fixture.

use std::fmt;

/// Somewhere an album can come from.
///
/// Both halves of Qobuz appear here, because Qobuz sells downloads *and* streams
/// subscriptions and the two land in different tiers off one API response. The
/// same album from [`Vendor::QobuzStore`] and [`Vendor::Qobuz`] is two offers,
/// not one — conflating them is the single easiest way to get this model wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Vendor {
    // Purchases, lossless.
    Bandcamp,
    QobuzStore,
    Bleep,
    Boomkat,
    PrestoMusic,
    SevenDigital,
    HdTracks,
    Beatport,
    JunoDownload,
    PhysicalNew,
    PhysicalUsed,
    // Purchases, lossy.
    ITunes,
    // Subscriptions.
    Qobuz,
    Tidal,
    AppleMusic,
    AmazonMusicHd,
    Deezer,
    Spotify,
    YouTubeMusic,
}

impl Vendor {
    /// Every vendor the ranking knows how to score, in a stable order.
    pub const ALL: [Vendor; 19] = [
        Vendor::Bandcamp,
        Vendor::QobuzStore,
        Vendor::Bleep,
        Vendor::Boomkat,
        Vendor::PrestoMusic,
        Vendor::SevenDigital,
        Vendor::HdTracks,
        Vendor::Beatport,
        Vendor::JunoDownload,
        Vendor::PhysicalNew,
        Vendor::PhysicalUsed,
        Vendor::ITunes,
        Vendor::Qobuz,
        Vendor::Tidal,
        Vendor::AppleMusic,
        Vendor::AmazonMusicHd,
        Vendor::Deezer,
        Vendor::Spotify,
        Vendor::YouTubeMusic,
    ];

    /// The name shown to a person.
    pub fn label(self) -> &'static str {
        match self {
            Vendor::Bandcamp => "Bandcamp",
            Vendor::QobuzStore => "Qobuz Store",
            Vendor::Bleep => "Bleep",
            Vendor::Boomkat => "Boomkat",
            Vendor::PrestoMusic => "Presto Music",
            Vendor::SevenDigital => "7digital",
            Vendor::HdTracks => "HDtracks",
            Vendor::Beatport => "Beatport",
            Vendor::JunoDownload => "Juno Download",
            Vendor::PhysicalNew => "CD (new)",
            Vendor::PhysicalUsed => "CD (used)",
            Vendor::ITunes => "iTunes Store",
            Vendor::Qobuz => "Qobuz",
            Vendor::Tidal => "Tidal",
            Vendor::AppleMusic => "Apple Music",
            Vendor::AmazonMusicHd => "Amazon Music HD",
            Vendor::Deezer => "Deezer",
            Vendor::Spotify => "Spotify",
            Vendor::YouTubeMusic => "YouTube Music",
        }
    }

    /// The key this vendor's weight overrides are written under in `config.toml`.
    pub fn key(self) -> &'static str {
        match self {
            Vendor::Bandcamp => "bandcamp",
            Vendor::QobuzStore => "qobuz_store",
            Vendor::Bleep => "bleep",
            Vendor::Boomkat => "boomkat",
            Vendor::PrestoMusic => "presto",
            Vendor::SevenDigital => "sevendigital",
            Vendor::HdTracks => "hdtracks",
            Vendor::Beatport => "beatport",
            Vendor::JunoDownload => "junodownload",
            Vendor::PhysicalNew => "physical_new",
            Vendor::PhysicalUsed => "physical_used",
            Vendor::ITunes => "itunes",
            Vendor::Qobuz => "qobuz",
            Vendor::Tidal => "tidal",
            Vendor::AppleMusic => "apple_music",
            Vendor::AmazonMusicHd => "amazon_music_hd",
            Vendor::Deezer => "deezer",
            Vendor::Spotify => "spotify",
            Vendor::YouTubeMusic => "youtube_music",
        }
    }

    pub fn from_key(key: &str) -> Option<Vendor> {
        Vendor::ALL.into_iter().find(|v| v.key() == key)
    }

    /// A one-line note about what this vendor pays the artist.
    ///
    /// Shown next to every result, because "higher payout beats lower" is the
    /// second of the four principles and a number with no explanation next to it
    /// is not reasoning.
    pub fn payout_note(self) -> &'static str {
        match self {
            Vendor::Bandcamp => "Roughly 82–90% reaches the artist",
            Vendor::QobuzStore
            | Vendor::Bleep
            | Vendor::Boomkat
            | Vendor::PrestoMusic
            | Vendor::SevenDigital
            | Vendor::HdTracks
            | Vendor::Beatport
            | Vendor::JunoDownload => "A download store's wholesale split, well above any stream",
            Vendor::PhysicalNew => "A new disc pays the label, and through it the artist",
            Vendor::PhysicalUsed => "Resale pays the artist nothing",
            Vendor::ITunes => "A store split, but on a 256 kbps file you cannot re-rip",
            Vendor::Qobuz => "The highest per-stream rate of any subscription service",
            Vendor::Tidal => "A high per-stream rate, below Qobuz",
            Vendor::AppleMusic | Vendor::Deezer => "A middling per-stream rate",
            Vendor::AmazonMusicHd => "A middling per-stream rate",
            Vendor::Spotify | Vendor::YouTubeMusic => "The lowest per-stream rate",
        }
    }
}

impl fmt::Display for Vendor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Whether you end up owning a file or renting access to one.
///
/// Every purchase vendor Sleeve implements delivers DRM-free files — Bandcamp,
/// the download stores, and iTunes, which dropped FairPlay from music in 2009.
/// There is deliberately no `Purchase { drm_free: false }` variant: nothing in
/// the tree produces one, and inventing it would put a branch in the tier
/// function that no test could ever reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acquisition {
    Purchase,
    Subscription,
}

/// What arrives, in audio terms.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Delivery {
    pub lossless: bool,
    pub bit_depth: Option<u8>,
    pub sample_rate_hz: Option<u32>,
    /// "FLAC", "AAC 256", "CD" — shown verbatim, never parsed.
    pub codec: Option<String>,
}

impl Delivery {
    pub fn lossy(codec: &str) -> Self {
        Delivery {
            lossless: false,
            codec: Some(codec.to_string()),
            ..Delivery::default()
        }
    }

    pub fn lossless(codec: &str, bit_depth: u8, sample_rate_hz: u32) -> Self {
        Delivery {
            lossless: true,
            bit_depth: Some(bit_depth),
            sample_rate_hz: Some(sample_rate_hz),
            codec: Some(codec.to_string()),
        }
    }

    /// "24-bit/96 kHz FLAC", or as much of it as is known.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let (Some(depth), Some(rate)) = (self.bit_depth, self.sample_rate_hz) {
            parts.push(format!("{}-bit/{} kHz", depth, rate as f64 / 1000.0));
        } else if let Some(depth) = self.bit_depth {
            parts.push(format!("{depth}-bit"));
        }
        if let Some(codec) = &self.codec {
            parts.push(codec.clone());
        }
        if parts.is_empty() {
            parts.push(if self.lossless {
                "Lossless".to_string()
            } else {
                "Lossy".to_string()
            });
        }
        parts.join(" ")
    }
}

/// Something standing between you and the music, beyond the price.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Friction {
    /// A disc, which is lossless but is not a file until you make it one.
    RequiresRipping,
    /// Not sold on its own — you buy the box or you buy nothing.
    BoxsetOnly,
    /// Not purchasable from the configured locale.
    ///
    /// This is not scored. A region-locked offer is not a slightly worse offer,
    /// it is not an offer, and [`super::score`] moves it out of the ranking
    /// entirely rather than nudging it down a few points.
    RegionLocked,
}

impl Friction {
    pub fn caveat(self) -> &'static str {
        match self {
            Friction::RequiresRipping => "Physical only — you rip it yourself",
            Friction::BoxsetOnly => "Sold only as part of a boxset",
            Friction::RegionLocked => "Not sold in your region",
        }
    }
}

/// How much this offer is actually known.
///
/// The distinction decides who wins when two sources describe the same vendor.
/// [`Provenance::Checked`] means a source asked that shop directly and got an
/// answer about this album today. [`Provenance::Indexed`] means somebody once
/// recorded that the shop sells it — true when written, and possibly not now.
///
/// A check always beats an index for the same vendor, including a *negative*
/// check: Bandcamp's API reports `is_purchasable: false` for records that
/// MusicBrainz still lists a Bandcamp purchase link for, and resurrecting one of
/// those from the index would put a tier-A offer that cannot be bought at the top
/// of the ranking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    Checked,
    Indexed,
}

/// A dynamic-range measurement for the master an offer is selling.
///
/// Carried on the offer rather than looked up during scoring, because the whole
/// difficulty of this signal is *attribution*: the Dynamic Range Database is
/// keyed by loose artist/album text and holds several entries per album, one per
/// pressing. Whoever assembles the offers decides which entry belongs to which
/// edition and records what it matched against; scoring then has nothing to
/// guess at. A wrong match here is worth up to 13 points, so `matched` is shown
/// in the caveat line — a misattribution should be visible, not silent.
#[derive(Debug, Clone, PartialEq)]
pub struct DynamicRange {
    pub dr: u8,
    /// The database row this came from, as text: "2000, CD, lossless".
    pub matched: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Price {
    pub amount: f64,
    /// ISO 4217, as the vendor reported it. Never converted — a converted price
    /// is a guess about an exchange rate presented as a fact about a shop.
    pub currency: String,
}

impl fmt::Display for Price {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let symbol = match self.currency.as_str() {
            "USD" => "$",
            "GBP" => "£",
            "EUR" => "€",
            "JPY" => "¥",
            other => return write!(f, "{:.2} {other}", self.amount),
        };
        write!(f, "{symbol}{:.2}", self.amount)
    }
}

/// One vendor's answer about one album.
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    pub vendor: Vendor,
    pub acquisition: Acquisition,
    pub delivery: Delivery,
    pub provenance: Provenance,
    pub price: Option<Price>,
    pub url: Option<String>,
    /// Which master or edition this vendor is selling, when it says.
    ///
    /// The thing that makes "multiple distinct masters exist" reportable rather
    /// than merely true.
    pub edition: Option<String>,
    /// The measured dynamic range of the master being sold, when one was matched
    /// to this specific edition. `None` is neutral and never a penalty.
    pub dynamic_range: Option<DynamicRange>,
    pub frictions: Vec<Friction>,
    /// Anything else a person should know before spending money.
    pub notes: Vec<String>,
}

impl Offer {
    pub fn new(vendor: Vendor, acquisition: Acquisition, delivery: Delivery) -> Self {
        Offer {
            vendor,
            acquisition,
            delivery,
            provenance: Provenance::Checked,
            price: None,
            url: None,
            edition: None,
            dynamic_range: None,
            frictions: Vec::new(),
            notes: Vec::new(),
        }
    }

    pub fn with_dynamic_range(mut self, dr: u8, matched: impl Into<String>) -> Self {
        self.dynamic_range = Some(DynamicRange {
            dr,
            matched: matched.into(),
        });
        self
    }

    /// Mark this as a link somebody recorded rather than a shop we asked.
    pub fn indexed(mut self) -> Self {
        self.provenance = Provenance::Indexed;
        self
    }

    pub fn with_price(mut self, amount: f64, currency: &str) -> Self {
        self.price = Some(Price {
            amount,
            currency: currency.to_string(),
        });
        self
    }

    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    pub fn with_edition(mut self, edition: impl Into<String>) -> Self {
        self.edition = Some(edition.into());
        self
    }

    pub fn with_friction(mut self, friction: Friction) -> Self {
        if !self.frictions.contains(&friction) {
            self.frictions.push(friction);
        }
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn is_region_locked(&self) -> bool {
        self.frictions.contains(&Friction::RegionLocked)
    }
}

/// Collapse offers describing the same thing, and drop links we know are dead.
///
/// Two sources routinely describe one shop: MusicBrainz indexes a purchase link
/// to Bandcamp, and Bandcamp's own API says what it costs. One row, not two.
///
/// `checked_absent` names vendors a live source asked and got a *no* from. Those
/// suppress an indexed link entirely, and that direction matters: Bandcamp
/// reports `is_purchasable: false` for records MusicBrainz still lists a Bandcamp
/// purchase link for. Without this, the index would resurrect a tier-A offer
/// nobody can buy and put it at the top of the ranking.
pub fn merge(offers: Vec<Offer>, checked_absent: &[Vendor]) -> Vec<Offer> {
    let mut kept: Vec<Offer> = Vec::new();

    for offer in offers {
        if offer.provenance == Provenance::Indexed && checked_absent.contains(&offer.vendor) {
            continue;
        }

        let existing = kept.iter().position(|other| {
            other.vendor == offer.vendor && other.acquisition == offer.acquisition
        });

        match existing {
            None => kept.push(offer),
            Some(index) => {
                if better(&offer, &kept[index]) {
                    // Keep whatever the loser knew and the winner does not: an
                    // indexed link often has a URL where a checked offer has a
                    // price, and a row with both is better than either.
                    let mut winner = offer;
                    if winner.url.is_none() {
                        winner.url = kept[index].url.clone();
                    }
                    if winner.price.is_none() {
                        winner.price = kept[index].price.clone();
                    }
                    kept[index] = winner;
                } else if kept[index].url.is_none() {
                    kept[index].url = offer.url;
                }
            }
        }
    }

    kept
}

/// Whether `candidate` is the better description of a shop than `incumbent`.
fn better(candidate: &Offer, incumbent: &Offer) -> bool {
    match (candidate.provenance, incumbent.provenance) {
        (Provenance::Checked, Provenance::Indexed) => true,
        (Provenance::Indexed, Provenance::Checked) => false,
        // Equally sourced: the one that knows the price knows more.
        _ => candidate.price.is_some() && incumbent.price.is_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_vendor_has_a_distinct_config_key_that_round_trips() {
        // The keys are the public surface of `config.toml`; two vendors sharing
        // one would make a weight override silently apply to the wrong shop.
        let mut keys: Vec<&str> = Vendor::ALL.iter().map(|v| v.key()).collect();
        keys.sort_unstable();
        let count = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), count, "duplicate vendor key");

        for vendor in Vendor::ALL {
            assert_eq!(Vendor::from_key(vendor.key()), Some(vendor));
        }
    }

    #[test]
    fn a_price_renders_in_its_own_currency_and_never_converts() {
        assert_eq!(
            Price {
                amount: 9.99,
                currency: "USD".into()
            }
            .to_string(),
            "$9.99"
        );
        assert_eq!(
            Price {
                amount: 12.0,
                currency: "GBP".into()
            }
            .to_string(),
            "£12.00"
        );
        // An unknown currency prints its code rather than guessing a symbol.
        assert_eq!(
            Price {
                amount: 150.0,
                currency: "SEK".into()
            }
            .to_string(),
            "150.00 SEK"
        );
    }

    fn purchase(vendor: Vendor) -> Offer {
        Offer::new(
            vendor,
            Acquisition::Purchase,
            Delivery::lossless("FLAC", 16, 44_100),
        )
    }

    #[test]
    fn a_checked_shop_beats_an_indexed_link_to_the_same_shop() {
        let merged = merge(
            vec![
                purchase(Vendor::Bandcamp)
                    .with_url("https://x.bandcamp.com/album/y")
                    .indexed(),
                purchase(Vendor::Bandcamp).with_price(8.0, "GBP"),
            ],
            &[],
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].provenance, Provenance::Checked);
        assert!(merged[0].price.is_some());
        // The URL only the index had is kept rather than thrown away with it.
        assert_eq!(
            merged[0].url.as_deref(),
            Some("https://x.bandcamp.com/album/y")
        );
    }

    #[test]
    fn a_shop_that_said_no_cannot_be_resurrected_by_an_index() {
        // Bandcamp's API returns `is_purchasable: false` for albums MusicBrainz
        // still has a Bandcamp purchase link for. The live answer wins, or the
        // ranking gains a top-placed offer nobody can buy.
        let merged = merge(
            vec![purchase(Vendor::Bandcamp)
                .with_url("https://radiohead.bandcamp.com/album/kid-a")
                .indexed()],
            &[Vendor::Bandcamp],
        );
        assert!(merged.is_empty());
    }

    #[test]
    fn a_shop_nobody_checked_survives_on_its_indexed_link_alone() {
        // Bleep refuses an HTTP client outright, so an index entry is the only
        // way it ever appears — and it is a real tier-A option.
        let merged = merge(vec![purchase(Vendor::Bleep).indexed()], &[Vendor::Bandcamp]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].vendor, Vendor::Bleep);
    }

    #[test]
    fn one_shop_selling_and_streaming_stays_two_offers() {
        // Qobuz is a shop and a subscription. Merging on vendor alone would
        // silently drop one of them.
        let merged = merge(
            vec![
                purchase(Vendor::QobuzStore).indexed(),
                Offer::new(
                    Vendor::Qobuz,
                    Acquisition::Subscription,
                    Delivery::lossless("FLAC", 24, 96_000),
                )
                .indexed(),
            ],
            &[],
        );
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn between_two_equally_sourced_offers_the_priced_one_wins() {
        let merged = merge(
            vec![
                purchase(Vendor::QobuzStore),
                purchase(Vendor::QobuzStore).with_price(13.49, "GBP"),
            ],
            &[],
        );
        assert_eq!(merged.len(), 1);
        assert!(merged[0].price.is_some());
    }

    #[test]
    fn delivery_describes_what_it_knows_and_no_more() {
        assert_eq!(
            Delivery::lossless("FLAC", 24, 96_000).describe(),
            "24-bit/96 kHz FLAC"
        );
        assert_eq!(Delivery::lossy("AAC 256").describe(), "AAC 256");
        assert_eq!(
            Delivery {
                lossless: true,
                ..Delivery::default()
            }
            .describe(),
            "Lossless"
        );
    }
}
