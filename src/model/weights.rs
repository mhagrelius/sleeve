//! The score table, and the overrides a person can put in `config.toml`.
//!
//! Everything here is data. [`super::score`] reads it and does arithmetic; it
//! never hard-codes a number. That is what makes "flag anything that produces
//! bad rankings" a question you can answer by editing a file and re-running the
//! tests rather than by reading the ranking code.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::offer::{Friction, Vendor};
use super::tier::Tier;

/// Base score per tier, indexed by [`Tier::index`].
pub const DEFAULT_BASE: [i32; 5] = [100, 50, 30, 10, 0];

/// How the ranking is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ordering {
    /// Sort by tier first, then by score inside the tier.
    ///
    /// The default, and the only ordering in which the four principles are
    /// guaranteed rather than hoped for. Under a plain numeric sort the top two
    /// principles hold only while the numbers happen to co-operate: a tier-B
    /// purchase of a brickwalled master scores 45 and a tier-C lossless stream
    /// of a good one scores 49, so the stream wins and "owning beats renting"
    /// quietly stops being true. Worse, weight overrides live in the same config
    /// file, so a person tuning a payout number could reintroduce that at any
    /// time and nothing would tell them.
    #[default]
    Lexicographic,
    /// Sort by total score alone, tier ignored except as a base.
    ///
    /// Here because it is what the scoring table literally describes, and
    /// because a person who wants it should not have to patch the source. The
    /// test suite asserts the inversions it permits, so choosing it is informed.
    Numeric,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Weights {
    pub ordering: Ordering,
    /// Base score for tiers A, B, C, D, E.
    pub base: [i32; 5],
    /// Per-vendor artist-payout bonus. Any vendor absent scores zero.
    pub payout: BTreeMap<String, i32>,
    pub lossless: i32,
    pub bit_depth_24: i32,
    /// Deliberately zero. Above 48 kHz is not audibly better, and the line is
    /// here so that the decision is visible in the config a person reads rather
    /// than implied by an absence.
    pub above_48khz: i32,
    pub dr_high: i32,
    pub dr_low: i32,
    pub dr_high_threshold: u8,
    pub dr_low_threshold: u8,
    pub requires_ripping: i32,
    pub boxset_only: i32,
}

impl Default for Weights {
    fn default() -> Self {
        Weights {
            ordering: Ordering::Lexicographic,
            base: DEFAULT_BASE,
            payout: default_payout(),
            lossless: 5,
            bit_depth_24: 2,
            above_48khz: 0,
            dr_high: 8,
            dr_low: -5,
            dr_high_threshold: 10,
            dr_low_threshold: 6,
            requires_ripping: -3,
            boxset_only: -5,
        }
    }
}

/// The payout table.
///
/// The brief gave four bands — direct-to-artist, indie hi-res store, and the
/// streaming rates — which between them do not cover every tier-A vendor. Two
/// gaps are filled here and both follow from "higher artist payout beats lower"
/// rather than from taste: physical resale is `0` because a used disc sends the
/// artist nothing, and a new disc is `+3` because it pays a label rather than
/// the artist directly. Every value is overridable.
fn default_payout() -> BTreeMap<String, i32> {
    let table = [
        (Vendor::Bandcamp, 15),
        (Vendor::QobuzStore, 5),
        (Vendor::Bleep, 5),
        (Vendor::Boomkat, 5),
        (Vendor::PrestoMusic, 5),
        (Vendor::SevenDigital, 5),
        (Vendor::HdTracks, 5),
        (Vendor::Beatport, 5),
        (Vendor::PhysicalNew, 3),
        (Vendor::PhysicalUsed, 0),
        (Vendor::ITunes, 0),
        (Vendor::Qobuz, 4),
        (Vendor::Tidal, 3),
        (Vendor::AppleMusic, 1),
        (Vendor::AmazonMusicHd, 1),
        (Vendor::Deezer, 1),
        (Vendor::Spotify, 0),
        (Vendor::YouTubeMusic, 0),
    ];
    table
        .into_iter()
        .map(|(vendor, value)| (vendor.key().to_string(), value))
        .collect()
}

impl Weights {
    pub fn base_for(&self, tier: Tier) -> i32 {
        self.base[tier.index()]
    }

    pub fn payout_for(&self, vendor: Vendor) -> i32 {
        self.payout.get(vendor.key()).copied().unwrap_or(0)
    }

    /// Scored frictions. [`Friction::RegionLocked`] is deliberately absent: it
    /// removes an offer from the ranking rather than costing it points.
    pub fn friction_for(&self, friction: Friction) -> i32 {
        match friction {
            Friction::RequiresRipping => self.requires_ripping,
            Friction::BoxsetOnly => self.boxset_only,
            Friction::RegionLocked => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_table_from_the_brief() {
        let w = Weights::default();
        assert_eq!(w.base, [100, 50, 30, 10, 0]);
        assert_eq!(w.payout_for(Vendor::Bandcamp), 15);
        assert_eq!(w.payout_for(Vendor::Qobuz), 4);
        assert_eq!(w.payout_for(Vendor::Spotify), 0);
        assert_eq!(w.lossless, 5);
        assert_eq!(w.bit_depth_24, 2);
        assert_eq!(w.above_48khz, 0);
        assert_eq!(w.dr_high, 8);
        assert_eq!(w.dr_low, -5);
    }

    #[test]
    fn an_unknown_vendor_key_in_the_payout_table_scores_zero_rather_than_panicking() {
        let w = Weights {
            payout: BTreeMap::new(),
            ..Weights::default()
        };
        for vendor in Vendor::ALL {
            assert_eq!(w.payout_for(vendor), 0);
        }
    }

    #[test]
    fn region_lock_is_not_a_scored_friction() {
        // It is an availability fact. If it ever starts costing points, an offer
        // you cannot buy is being ranked against ones you can.
        assert_eq!(Weights::default().friction_for(Friction::RegionLocked), 0);
    }

    #[test]
    fn overrides_parse_from_toml_and_leave_the_rest_at_their_defaults() {
        let w: Weights = toml::from_str(
            r#"
            ordering = "numeric"
            lossless = 9
            [payout]
            bandcamp = 25
            "#,
        )
        .expect("weights parse");
        assert_eq!(w.ordering, Ordering::Numeric);
        assert_eq!(w.lossless, 9);
        assert_eq!(w.payout_for(Vendor::Bandcamp), 25);
        // An override of one payout key replaces the table, so everything else
        // falls to zero. Documented here because it is surprising, and because
        // the alternative — merging — would make it impossible to set a value
        // back to zero.
        assert_eq!(w.payout_for(Vendor::Qobuz), 0);
        assert_eq!(w.base, DEFAULT_BASE);
    }
}
