//! The five tiers, and the only rule that decides which one an offer is in.
//!
//! Tier is a function of two facts and nothing else: do you end up owning a
//! file, and is that file lossless. Not the vendor's marketing, not the bit
//! depth, not the price. Deriving it from anything richer is how a 24/192 stream
//! ends up above a 16/44.1 purchase.

use super::offer::{Acquisition, Offer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// DRM-free purchase, lossless.
    A,
    /// DRM-free purchase, lossy.
    B,
    /// Subscription streaming, lossless or hi-res.
    C,
    /// Subscription streaming, lossy.
    D,
    /// Not legitimately available.
    ///
    /// Never attached to an offer — an offer in hand is by definition available.
    /// This is a statement about an album for which no vendor returned anything,
    /// and it lives on [`super::verdict::Verdict`] rather than in the ranking.
    E,
}

impl Tier {
    pub const RANKED: [Tier; 4] = [Tier::A, Tier::B, Tier::C, Tier::D];

    pub fn letter(self) -> &'static str {
        match self {
            Tier::A => "A",
            Tier::B => "B",
            Tier::C => "C",
            Tier::D => "D",
            Tier::E => "E",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Tier::A => "Own it, lossless",
            Tier::B => "Own it, lossy",
            Tier::C => "Rent it, lossless",
            Tier::D => "Rent it, lossy",
            Tier::E => "Not legitimately available",
        }
    }

    /// Index into the base-score table.
    pub fn index(self) -> usize {
        match self {
            Tier::A => 0,
            Tier::B => 1,
            Tier::C => 2,
            Tier::D => 3,
            Tier::E => 4,
        }
    }
}

/// Which tier an offer is in.
pub fn tier_of(offer: &Offer) -> Tier {
    match (offer.acquisition, offer.delivery.lossless) {
        (Acquisition::Purchase, true) => Tier::A,
        (Acquisition::Purchase, false) => Tier::B,
        (Acquisition::Subscription, true) => Tier::C,
        (Acquisition::Subscription, false) => Tier::D,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::offer::{Delivery, Vendor};

    #[test]
    fn tier_follows_ownership_first_and_quality_second() {
        let cases = [
            (
                Vendor::Bandcamp,
                Acquisition::Purchase,
                Delivery::lossless("FLAC", 16, 44_100),
                Tier::A,
            ),
            (
                Vendor::ITunes,
                Acquisition::Purchase,
                Delivery::lossy("AAC 256"),
                Tier::B,
            ),
            (
                Vendor::Qobuz,
                Acquisition::Subscription,
                Delivery::lossless("FLAC", 24, 192_000),
                Tier::C,
            ),
            (
                Vendor::YouTubeMusic,
                Acquisition::Subscription,
                Delivery::lossy("Opus"),
                Tier::D,
            ),
        ];
        for (vendor, acquisition, delivery, expected) in cases {
            let offer = Offer::new(vendor, acquisition, delivery);
            assert_eq!(tier_of(&offer), expected, "{vendor}");
        }
    }

    #[test]
    fn a_hi_res_stream_is_a_lower_tier_than_a_cd_quality_purchase() {
        // The inversion the whole model exists to prevent, asserted at the tier
        // level where it is a structural fact rather than an arithmetic accident.
        let stream = Offer::new(
            Vendor::Qobuz,
            Acquisition::Subscription,
            Delivery::lossless("FLAC", 24, 192_000),
        );
        let purchase = Offer::new(
            Vendor::Bandcamp,
            Acquisition::Purchase,
            Delivery::lossless("FLAC", 16, 44_100),
        );
        assert!(tier_of(&purchase) < tier_of(&stream));
    }

    #[test]
    fn tier_ordering_is_a_before_b_before_c_before_d_before_e() {
        assert!(Tier::A < Tier::B);
        assert!(Tier::B < Tier::C);
        assert!(Tier::C < Tier::D);
        assert!(Tier::D < Tier::E);
    }
}
