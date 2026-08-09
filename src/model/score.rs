//! The ranking engine.
//!
//! Four principles, in strict priority order, and the code is arranged so that
//! the first two are structural rather than arithmetic:
//!
//! 1. Owning DRM-free files beats renting. A purchase outranks any stream.
//! 2. Higher artist payout beats lower.
//! 3. Master quality beats format specs. Bit depth and sample rate are
//!    tiebreakers and are weighted so small they cannot be anything else.
//! 4. Actually available beats theoretically ideal.
//!
//! Principles 1 and 2 are enforced by sorting on [`Tier`] before score under the
//! default [`Ordering::Lexicographic`], so no combination of weights — including
//! ones a person writes into `config.toml` — can invert them. Principle 4 is
//! enforced by [`Friction::RegionLocked`] removing an offer from the ranking
//! rather than docking it points.
//!
//! Nothing here formats anything for display beyond plain strings, and nothing
//! here imports a UI type. The output is a data structure the UI renders.

use super::offer::{Offer, Price, Provenance};
use super::tier::{tier_of, Tier};
use super::weights::{Ordering, Weights};

/// One line of the arithmetic, kept so the UI can show its working.
///
/// A ranking a person cannot audit is a ranking they have to take on trust, and
/// the whole point of this tool is that they should not have to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    pub label: String,
    pub delta: i32,
}

impl Component {
    fn new(label: impl Into<String>, delta: i32) -> Self {
        Component {
            label: label.into(),
            delta,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScoredOffer {
    pub offer: Offer,
    pub tier: Tier,
    pub score: i32,
    pub components: Vec<Component>,
    pub caveats: Vec<String>,
}

impl ScoredOffer {
    pub fn price(&self) -> Option<&Price> {
        self.offer.price.as_ref()
    }
}

/// The ranked list, and the offers that were excluded from it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ranked {
    pub ranked: Vec<ScoredOffer>,
    /// Offers that exist but cannot be bought from the configured locale.
    ///
    /// Kept and shown, because "this is sold, but not to you" is useful and
    /// different from "this does not exist". Never interleaved with the ranking.
    pub unavailable_here: Vec<ScoredOffer>,
}

impl Ranked {
    pub fn best(&self) -> Option<&ScoredOffer> {
        self.ranked.first()
    }

    /// The cheapest offer that is actually a purchase.
    ///
    /// A subscription has no price in the sense meant here — the monthly fee is
    /// not the cost of this album — so streams are excluded rather than counted
    /// as free.
    pub fn cheapest(&self) -> Option<&ScoredOffer> {
        self.ranked
            .iter()
            .filter(|scored| scored.offer.price.is_some())
            .min_by(|a, b| {
                let (a_price, b_price) = (&a.offer.price, &b.offer.price);
                match (a_price, b_price) {
                    (Some(x), Some(y)) if x.currency == y.currency => x
                        .amount
                        .partial_cmp(&y.amount)
                        .unwrap_or(std::cmp::Ordering::Equal),
                    // Prices in different currencies are not comparable without
                    // inventing an exchange rate, so the tier order stands in.
                    _ => a.tier.cmp(&b.tier),
                }
            })
    }

    pub fn is_empty(&self) -> bool {
        self.ranked.is_empty() && self.unavailable_here.is_empty()
    }
}

/// Score one offer, showing the working.
pub fn score_offer(offer: &Offer, weights: &Weights) -> ScoredOffer {
    let tier = tier_of(offer);
    let mut components = Vec::new();
    let mut caveats = Vec::new();

    let base = weights.base_for(tier);
    components.push(Component::new(
        format!("Tier {} — {}", tier.letter(), tier.description()),
        base,
    ));

    let payout = weights.payout_for(offer.vendor);
    if payout != 0 {
        components.push(Component::new(offer.vendor.payout_note(), payout));
    }

    if offer.delivery.lossless && weights.lossless != 0 {
        components.push(Component::new("Lossless", weights.lossless));
    }
    if offer.delivery.bit_depth.is_some_and(|depth| depth >= 24) && weights.bit_depth_24 != 0 {
        components.push(Component::new("24-bit", weights.bit_depth_24));
    }
    if offer
        .delivery
        .sample_rate_hz
        .is_some_and(|rate| rate > 48_000)
        && weights.above_48khz != 0
    {
        components.push(Component::new("Above 48 kHz", weights.above_48khz));
    }

    if let Some(dr) = &offer.dynamic_range {
        let delta = if dr.dr >= weights.dr_high_threshold {
            weights.dr_high
        } else if dr.dr <= weights.dr_low_threshold {
            weights.dr_low
        } else {
            0
        };
        if delta != 0 {
            components.push(Component::new(format!("DR{}", dr.dr), delta));
        }
        caveats.push(format!("DR{} measured on {}", dr.dr, dr.matched));
    }

    // An indexed row is a link somebody recorded, not a shop we asked today. It
    // ranks on equal terms — the tier is the tier — but a person deciding where
    // to spend money is owed the difference between "£11.00" and "we know they
    // stock it".
    if offer.provenance == Provenance::Indexed {
        caveats.push(if offer.price.is_none() {
            "Listed by MusicBrainz — price not known without visiting".to_string()
        } else {
            "Listed by MusicBrainz rather than checked just now".to_string()
        });
    }

    for friction in &offer.frictions {
        let delta = weights.friction_for(*friction);
        if delta != 0 {
            components.push(Component::new(friction.caveat(), delta));
        }
        caveats.push(friction.caveat().to_string());
    }

    caveats.extend(offer.notes.iter().cloned());

    let score = components.iter().map(|component| component.delta).sum();
    ScoredOffer {
        offer: offer.clone(),
        tier,
        score,
        components,
        caveats,
    }
}

/// Score and order every offer.
pub fn rank(offers: &[Offer], weights: &Weights) -> Ranked {
    let (locked, available): (Vec<_>, Vec<_>) = offers
        .iter()
        .map(|offer| score_offer(offer, weights))
        .partition(|scored| scored.offer.is_region_locked());

    let mut ranked = available;
    let mut unavailable_here = locked;
    sort(&mut ranked, weights.ordering);
    sort(&mut unavailable_here, weights.ordering);

    Ranked {
        ranked,
        unavailable_here,
    }
}

fn sort(offers: &mut [ScoredOffer], ordering: Ordering) {
    offers.sort_by(|a, b| {
        let primary = match ordering {
            Ordering::Lexicographic => a.tier.cmp(&b.tier).then(b.score.cmp(&a.score)),
            Ordering::Numeric => b.score.cmp(&a.score).then(a.tier.cmp(&b.tier)),
        };
        // Vendor order last so that equal offers come out in the same order on
        // every run. A list that reshuffles between identical searches looks
        // broken, and it makes a fixture test flaky for no reason.
        primary.then(a.offer.vendor.cmp(&b.offer.vendor))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::offer::{Acquisition, Delivery, Friction, Vendor};

    fn purchase(vendor: Vendor, delivery: Delivery) -> Offer {
        Offer::new(vendor, Acquisition::Purchase, delivery)
    }

    fn stream(vendor: Vendor, delivery: Delivery) -> Offer {
        Offer::new(vendor, Acquisition::Subscription, delivery)
    }

    #[test]
    fn a_bandcamp_flac_scores_the_brief_s_arithmetic() {
        // 100 base + 15 payout + 5 lossless = 120.
        let offer = purchase(Vendor::Bandcamp, Delivery::lossless("FLAC", 16, 44_100));
        assert_eq!(score_offer(&offer, &Weights::default()).score, 120);
    }

    #[test]
    fn a_hi_res_qobuz_stream_of_a_great_master_scores_the_brief_s_arithmetic() {
        // 30 base + 4 payout + 5 lossless + 2 24-bit + 0 above-48k + 8 DR = 49.
        let offer = stream(Vendor::Qobuz, Delivery::lossless("FLAC", 24, 192_000))
            .with_dynamic_range(14, "2000, CD, lossless");
        assert_eq!(score_offer(&offer, &Weights::default()).score, 49);
    }

    #[test]
    fn above_48khz_contributes_nothing() {
        let cd = stream(Vendor::Tidal, Delivery::lossless("FLAC", 24, 44_100));
        let hi_res = stream(Vendor::Tidal, Delivery::lossless("FLAC", 24, 192_000));
        let weights = Weights::default();
        assert_eq!(
            score_offer(&cd, &weights).score,
            score_offer(&hi_res, &weights).score
        );
    }

    #[test]
    fn an_indexed_offer_ranks_normally_but_says_where_it_came_from() {
        // It is still tier A and still scores 105 — an editor's link is evidence
        // that a shop sells the record. What changes is that the row admits it
        // was not priced today.
        let indexed = purchase(Vendor::Bleep, Delivery::lossless("FLAC", 16, 44_100))
            .indexed()
            .with_url("https://bleep.com/release/36698");
        let scored = score_offer(&indexed, &Weights::default());

        assert_eq!(scored.tier, Tier::A);
        assert_eq!(scored.score, 110);
        assert!(scored
            .caveats
            .iter()
            .any(|caveat| caveat.contains("price not known")));
    }

    #[test]
    fn a_checked_offer_carries_no_provenance_caveat() {
        let checked = purchase(Vendor::Bandcamp, Delivery::lossless("FLAC", 16, 44_100))
            .with_price(8.0, "GBP");
        let scored = score_offer(&checked, &Weights::default());
        assert!(!scored
            .caveats
            .iter()
            .any(|caveat| caveat.contains("MusicBrainz")));
    }

    #[test]
    fn a_missing_dr_entry_is_neutral_and_never_a_penalty() {
        let weights = Weights::default();
        let without = purchase(Vendor::QobuzStore, Delivery::lossless("FLAC", 24, 96_000));
        let with_good = without
            .clone()
            .with_dynamic_range(12, "2016, WEB, lossless");
        let with_bad = without.clone().with_dynamic_range(4, "2016, WEB, lossless");

        let neutral = score_offer(&without, &weights).score;
        assert!(score_offer(&with_good, &weights).score > neutral);
        assert!(score_offer(&with_bad, &weights).score < neutral);
    }

    #[test]
    fn a_dr_match_is_always_reported_even_when_it_scores_zero() {
        // DR 8 sits between the thresholds and moves nothing, but the person
        // still needs to see which pressing was measured — that is how a bad
        // match becomes visible instead of silently shifting a ranking.
        let offer = purchase(Vendor::Bandcamp, Delivery::lossless("FLAC", 16, 44_100))
            .with_dynamic_range(8, "2000, CD, lossless");
        let scored = score_offer(&offer, &Weights::default());
        assert!(!scored.components.iter().any(|c| c.label.starts_with("DR")));
        assert!(scored
            .caveats
            .iter()
            .any(|caveat| caveat == "DR8 measured on 2000, CD, lossless"));
    }

    // -- the inversions -----------------------------------------------------

    #[test]
    fn a_purchase_always_outranks_a_stream_however_the_weights_fall() {
        // The worst purchase the model can produce against the best stream it
        // can produce. Under the default ordering the purchase wins because tier
        // is the primary key, and no arithmetic can change that.
        let weights = Weights::default();
        let worst_purchase = purchase(Vendor::ITunes, Delivery::lossy("AAC 256"))
            .with_dynamic_range(3, "2011 remaster, CD, lossless");
        let best_stream = stream(Vendor::Qobuz, Delivery::lossless("FLAC", 24, 192_000))
            .with_dynamic_range(14, "2000, CD, lossless");

        let ranked = rank(&[best_stream.clone(), worst_purchase.clone()], &weights);
        assert_eq!(ranked.ranked[0].offer.vendor, Vendor::ITunes);
        assert_eq!(ranked.ranked[1].offer.vendor, Vendor::Qobuz);

        // And the scores really do invert, which is exactly why the sort cannot
        // be numeric. 45 against 49.
        assert!(
            score_offer(&worst_purchase, &weights).score
                < score_offer(&best_stream, &weights).score
        );
    }

    #[test]
    fn numeric_ordering_permits_the_inversion_lexicographic_prevents() {
        // Documenting the cost of the opt-out rather than hiding it: a person who
        // sets `ordering = "numeric"` gets the brief's literal table, including
        // a lossless stream above a lossy purchase.
        let weights = Weights {
            ordering: Ordering::Numeric,
            ..Weights::default()
        };
        let purchase = purchase(Vendor::ITunes, Delivery::lossy("AAC 256"))
            .with_dynamic_range(3, "2011 remaster, CD, lossless");
        let stream = stream(Vendor::Qobuz, Delivery::lossless("FLAC", 24, 192_000))
            .with_dynamic_range(14, "2000, CD, lossless");
        let ranked = rank(&[purchase, stream], &weights);
        assert_eq!(ranked.ranked[0].offer.vendor, Vendor::Qobuz);
    }

    #[test]
    fn no_weight_override_can_lift_a_stream_above_a_purchase() {
        // The property the default ordering exists to guarantee. Payout is
        // cranked absurdly in the streams' favour and the purchase still wins.
        let mut weights = Weights::default();
        for vendor in [Vendor::Spotify, Vendor::Qobuz, Vendor::Tidal] {
            weights.payout.insert(vendor.key().to_string(), 10_000);
        }
        let offers = vec![
            stream(Vendor::Spotify, Delivery::lossless("FLAC", 24, 44_100)),
            stream(Vendor::Qobuz, Delivery::lossless("FLAC", 24, 192_000)),
            purchase(Vendor::PhysicalUsed, Delivery::lossless("CD", 16, 44_100))
                .with_friction(Friction::RequiresRipping),
        ];
        let ranked = rank(&offers, &weights);
        assert_eq!(ranked.ranked[0].offer.vendor, Vendor::PhysicalUsed);
    }

    #[test]
    fn a_region_locked_offer_leaves_the_ranking_rather_than_losing_points() {
        let offers = vec![
            purchase(Vendor::Bandcamp, Delivery::lossless("FLAC", 16, 44_100))
                .with_friction(Friction::RegionLocked),
            stream(Vendor::Spotify, Delivery::lossy("Ogg 320")),
        ];
        let ranked = rank(&offers, &Weights::default());

        assert_eq!(ranked.ranked.len(), 1);
        assert_eq!(ranked.ranked[0].offer.vendor, Vendor::Spotify);
        assert_eq!(ranked.unavailable_here.len(), 1);
        // The score is kept and shown — it is a real offer, just not to us.
        assert_eq!(ranked.unavailable_here[0].score, 120);
    }

    #[test]
    fn payout_orders_the_streaming_tier_the_way_the_brief_asks() {
        let offers: Vec<Offer> = [
            Vendor::Spotify,
            Vendor::Deezer,
            Vendor::Tidal,
            Vendor::Qobuz,
            Vendor::AppleMusic,
        ]
        .into_iter()
        .map(|vendor| stream(vendor, Delivery::lossless("FLAC", 16, 44_100)))
        .collect();

        let ranked = rank(&offers, &Weights::default());
        let order: Vec<Vendor> = ranked.ranked.iter().map(|s| s.offer.vendor).collect();
        assert_eq!(order[0], Vendor::Qobuz);
        assert_eq!(order[1], Vendor::Tidal);
        assert_eq!(order[4], Vendor::Spotify);
    }

    #[test]
    fn bandcamp_outranks_a_used_cd_which_outranks_itunes() {
        let offers = vec![
            purchase(Vendor::ITunes, Delivery::lossy("AAC 256")).with_price(9.99, "GBP"),
            purchase(Vendor::PhysicalUsed, Delivery::lossless("CD", 16, 44_100))
                .with_friction(Friction::RequiresRipping),
            purchase(Vendor::Bandcamp, Delivery::lossless("FLAC", 24, 96_000)),
        ];
        let order: Vec<Vendor> = rank(&offers, &Weights::default())
            .ranked
            .iter()
            .map(|s| s.offer.vendor)
            .collect();
        assert_eq!(
            order,
            vec![Vendor::Bandcamp, Vendor::PhysicalUsed, Vendor::ITunes]
        );
    }

    #[test]
    fn the_order_is_stable_for_offers_that_score_identically() {
        let build = || {
            vec![
                stream(Vendor::Deezer, Delivery::lossless("FLAC", 16, 44_100)),
                stream(Vendor::AppleMusic, Delivery::lossless("FLAC", 16, 44_100)),
            ]
        };
        let first = rank(&build(), &Weights::default());
        let mut reversed = build();
        reversed.reverse();
        let second = rank(&reversed, &Weights::default());
        assert_eq!(first, second);
    }

    #[test]
    fn cheapest_ignores_streams_and_refuses_to_compare_across_currencies() {
        let offers = vec![
            purchase(Vendor::Bandcamp, Delivery::lossless("FLAC", 16, 44_100))
                .with_price(12.0, "GBP"),
            purchase(Vendor::ITunes, Delivery::lossy("AAC 256")).with_price(7.99, "GBP"),
            stream(Vendor::Spotify, Delivery::lossy("Ogg 320")),
        ];
        let ranked = rank(&offers, &Weights::default());
        assert_eq!(ranked.cheapest().unwrap().offer.vendor, Vendor::ITunes);
        assert_eq!(ranked.best().unwrap().offer.vendor, Vendor::Bandcamp);
    }

    #[test]
    fn an_empty_input_ranks_to_nothing_rather_than_panicking() {
        let ranked = rank(&[], &Weights::default());
        assert!(ranked.is_empty());
        assert!(ranked.best().is_none());
        assert!(ranked.cheapest().is_none());
    }
}
