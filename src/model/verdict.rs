//! The whole answer to one lookup: the ranking, what is odd about it, and a
//! sentence a person can act on.
//!
//! The recommendation is assembled here rather than in a widget so that it is
//! testable and so that the UI has nothing to decide. Everything below returns
//! plain strings and plain data.

use std::collections::BTreeSet;

use super::album::Release;
use super::offer::Vendor;
use super::score::{Ranked, ScoredOffer};
use super::source::{Reason, SourceId};
use super::tier::Tier;

/// Something about the results a person should be told rather than left to
/// notice.
#[derive(Debug, Clone, PartialEq)]
pub enum Conflict {
    /// Different sources are selling different masters of the same album.
    ///
    /// The case the brief cares most about: a remaster with worse dynamics
    /// sitting alongside the original, at the same price, under the same title.
    MultipleMasters { editions: Vec<EditionNote> },
    /// The best-scoring option and the cheapest one are not the same option.
    BestIsNotCheapest {
        best: Vendor,
        best_price: Option<String>,
        cheapest: Vendor,
        cheapest_price: String,
    },
    /// A remaster is measurably more compressed than the original it replaced.
    RemasterIsFlatter {
        original: EditionNote,
        remaster: EditionNote,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EditionNote {
    pub edition: String,
    pub dr: Option<u8>,
    pub vendors: Vec<Vendor>,
}

/// Why an album has no legitimate purchase path.
#[derive(Debug, Clone, PartialEq)]
pub struct NotAvailable {
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Verdict {
    /// The release this verdict is about, when one was chosen.
    pub release: Option<Release>,
    pub ranked: Ranked,
    /// Set when nothing anywhere sells or streams this. Tier E is a statement
    /// about an album, not a row in a list, so it lives here rather than
    /// occupying a rank.
    pub not_available: Option<NotAvailable>,
    pub conflicts: Vec<Conflict>,
    /// Sources that were asked and could not answer.
    ///
    /// Present so that an incomplete answer never reads as a complete one. With
    /// Bandcamp missing, most rankings lose their winner and look fine.
    ///
    /// A source that was never asked because its key is unset never appears
    /// here: that is a standing property of the configuration, identical under
    /// every lookup, and repeating it under each result says nothing.
    pub unchecked: Vec<(SourceId, String)>,
    pub recommendation: String,
}

impl Verdict {
    /// Build the verdict from a finished ranking.
    pub fn assemble(
        release: Option<Release>,
        ranked: Ranked,
        gaps: Vec<(SourceId, Reason)>,
    ) -> Self {
        let unchecked = worth_reporting(gaps);
        let conflicts = detect_conflicts(&ranked);
        let not_available = if ranked.is_empty() {
            Some(NotAvailable {
                reason: no_source_reason(&unchecked),
            })
        } else {
            None
        };
        let recommendation = recommend(&ranked, &conflicts, not_available.as_ref(), &unchecked);
        Verdict {
            release,
            ranked,
            not_available,
            conflicts,
            unchecked,
            recommendation,
        }
    }

    pub fn tier(&self) -> Tier {
        self.ranked.best().map(|best| best.tier).unwrap_or(Tier::E)
    }
}

/// Drop the gaps that are not about this lookup, and flatten the rest to text.
fn worth_reporting(gaps: Vec<(SourceId, Reason)>) -> Vec<(SourceId, String)> {
    gaps.into_iter()
        // An unset optional credential is not a fact about this album. It is the
        // same under every lookup, the person has already declined to act on it,
        // and it is written down in the config file they declined it in. Saying
        // "Not checked: Qobuz Store — the ranking may be missing a better
        // option" under every single result conveys nothing and is the first
        // thing anyone notices. Genuine failures — a timeout, a rate limit, a
        // shop that changed shape — are about this lookup and are always shown.
        .filter(|(_, reason)| !matches!(reason, Reason::NotConfigured(_)))
        .map(|(source, reason)| (source, reason.to_string()))
        .collect()
}

fn no_source_reason(unchecked: &[(SourceId, String)]) -> String {
    if unchecked.is_empty() {
        "No shop or streaming service Sleeve can reach sells or streams this. \
         Some albums only ever shipped as a physical or bundled item and were \
         never released on their own."
            .to_string()
    } else {
        format!(
            "Nothing was found, but {} could not be reached — so this may be a \
             gap in the lookup rather than a gap in the world.",
            list(
                &unchecked
                    .iter()
                    .map(|(id, _)| id.label())
                    .collect::<Vec<_>>()
            )
        )
    }
}

fn detect_conflicts(ranked: &Ranked) -> Vec<Conflict> {
    let mut conflicts = Vec::new();

    let editions = group_editions(&ranked.ranked);
    if editions.len() > 1 {
        conflicts.push(Conflict::MultipleMasters {
            editions: editions.clone(),
        });

        // A remaster that is flatter than the original it replaced is the
        // specific case worth naming, not just "there are two of these".
        let original = editions
            .iter()
            .find(|note| !looks_like_remaster(&note.edition));
        let remaster = editions
            .iter()
            .find(|note| looks_like_remaster(&note.edition));
        if let (Some(original), Some(remaster)) = (original, remaster) {
            if let (Some(a), Some(b)) = (original.dr, remaster.dr) {
                if b < a {
                    conflicts.push(Conflict::RemasterIsFlatter {
                        original: original.clone(),
                        remaster: remaster.clone(),
                    });
                }
            }
        }
    }

    if let (Some(best), Some(cheapest)) = (ranked.best(), ranked.cheapest()) {
        if best.offer.vendor != cheapest.offer.vendor {
            if let Some(price) = &cheapest.offer.price {
                conflicts.push(Conflict::BestIsNotCheapest {
                    best: best.offer.vendor,
                    best_price: best.offer.price.as_ref().map(|p| p.to_string()),
                    cheapest: cheapest.offer.vendor,
                    cheapest_price: price.to_string(),
                });
            }
        }
    }

    conflicts
}

fn group_editions(offers: &[ScoredOffer]) -> Vec<EditionNote> {
    let mut notes: Vec<EditionNote> = Vec::new();
    for scored in offers {
        let Some(edition) = &scored.offer.edition else {
            continue;
        };
        let dr = scored.offer.dynamic_range.as_ref().map(|entry| entry.dr);
        match notes.iter_mut().find(|note| &note.edition == edition) {
            Some(note) => {
                if !note.vendors.contains(&scored.offer.vendor) {
                    note.vendors.push(scored.offer.vendor);
                }
                note.dr = note.dr.or(dr);
            }
            None => notes.push(EditionNote {
                edition: edition.clone(),
                dr,
                vendors: vec![scored.offer.vendor],
            }),
        }
    }
    notes
}

fn looks_like_remaster(edition: &str) -> bool {
    let lowered = edition.to_lowercase();
    ["remaster", "remastered", "anniversary", "deluxe"]
        .iter()
        .any(|needle| lowered.contains(needle))
}

fn recommend(
    ranked: &Ranked,
    conflicts: &[Conflict],
    not_available: Option<&NotAvailable>,
    unchecked: &[(SourceId, String)],
) -> String {
    if let Some(missing) = not_available {
        return missing.reason.clone();
    }

    let Some(best) = ranked.best() else {
        return String::new();
    };

    let mut sentences = Vec::new();
    let price = best
        .offer
        .price
        .as_ref()
        .map(|price| format!(" for {price}"))
        .unwrap_or_default();

    sentences.push(match best.tier {
        Tier::A => format!(
            "Buy it from {}{}. {} — and you keep the files.",
            best.offer.vendor,
            price,
            best.offer.delivery.describe()
        ),
        Tier::B => format!(
            "{}{} is the only way to own this one; it is {}, not lossless.",
            best.offer.vendor,
            price,
            best.offer.delivery.describe()
        ),
        Tier::C | Tier::D => format!(
            "No one sells this as files, so {} is the best you can do — {}, \
             and only while it stays in the catalogue.",
            best.offer.vendor,
            best.offer.delivery.describe()
        ),
        Tier::E => String::new(),
    });

    for conflict in conflicts {
        match conflict {
            Conflict::BestIsNotCheapest {
                cheapest,
                cheapest_price,
                ..
            } => sentences.push(format!(
                "{cheapest} is cheaper at {cheapest_price}, if the difference matters more \
                 to you than the ranking does."
            )),
            Conflict::RemasterIsFlatter {
                original, remaster, ..
            } => sentences.push(format!(
                "Watch which master you get: the {} measures DR{}, against DR{} for the {}.",
                remaster.edition,
                remaster.dr.unwrap_or(0),
                original.dr.unwrap_or(0),
                original.edition
            )),
            Conflict::MultipleMasters { editions } => {
                if !conflicts
                    .iter()
                    .any(|other| matches!(other, Conflict::RemasterIsFlatter { .. }))
                {
                    sentences.push(format!(
                        "There is more than one master in circulation ({}), so check which \
                         one you are buying.",
                        list(
                            &editions
                                .iter()
                                .map(|e| e.edition.as_str())
                                .collect::<Vec<_>>()
                        )
                    ));
                }
            }
        }
    }

    if !ranked.unavailable_here.is_empty() {
        let vendors: BTreeSet<&str> = ranked
            .unavailable_here
            .iter()
            .map(|scored| scored.offer.vendor.label())
            .collect();
        sentences.push(format!(
            "{} {} it, but not to your region.",
            list(&vendors.into_iter().collect::<Vec<_>>()),
            if ranked.unavailable_here.len() == 1 {
                "sells"
            } else {
                "sell"
            }
        ));
    }

    if !unchecked.is_empty() {
        sentences.push(format!(
            "Not checked: {}. The ranking may be missing a better option.",
            list(
                &unchecked
                    .iter()
                    .map(|(id, _)| id.label())
                    .collect::<Vec<_>>()
            )
        ));
    }

    sentences.retain(|sentence| !sentence.is_empty());
    sentences.join(" ")
}

/// "a", "a and b", "a, b and c".
fn list(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::offer::{Acquisition, Delivery, Friction, Offer};
    use crate::model::score::rank;
    use crate::model::weights::Weights;

    fn purchase(vendor: Vendor, delivery: Delivery) -> Offer {
        Offer::new(vendor, Acquisition::Purchase, delivery)
    }

    #[test]
    fn nothing_anywhere_is_reported_as_tier_e_with_a_reason_and_no_ranking() {
        let verdict = Verdict::assemble(None, rank(&[], &Weights::default()), Vec::new());
        assert_eq!(verdict.tier(), Tier::E);
        assert!(verdict.ranked.ranked.is_empty());
        let reason = &verdict.not_available.as_ref().unwrap().reason;
        assert!(reason.contains("never released on their own"));
        // Nothing in the answer may point anywhere unauthorised, so there is no
        // link and no suggestion — only the statement.
        assert_eq!(&verdict.recommendation, reason);
    }

    #[test]
    fn nothing_found_but_sources_down_says_so_rather_than_claiming_it_does_not_exist() {
        let verdict = Verdict::assemble(
            None,
            rank(&[], &Weights::default()),
            vec![(SourceId::Bandcamp, Reason::Timeout)],
        );
        assert!(verdict
            .recommendation
            .contains("a gap in the lookup rather than a gap in the world"));
    }

    #[test]
    fn the_best_and_the_cheapest_differing_is_surfaced_explicitly() {
        let offers = vec![
            purchase(Vendor::Bandcamp, Delivery::lossless("FLAC", 24, 96_000))
                .with_price(14.0, "GBP"),
            purchase(Vendor::ITunes, Delivery::lossy("AAC 256")).with_price(7.99, "GBP"),
        ];
        let verdict = Verdict::assemble(None, rank(&offers, &Weights::default()), Vec::new());
        assert!(verdict.conflicts.iter().any(|conflict| matches!(
            conflict,
            Conflict::BestIsNotCheapest {
                best: Vendor::Bandcamp,
                cheapest: Vendor::ITunes,
                ..
            }
        )));
        assert!(verdict.recommendation.contains("£7.99"));
    }

    #[test]
    fn a_flatter_remaster_alongside_the_original_is_named_with_both_dr_values() {
        let offers = vec![
            purchase(Vendor::Bandcamp, Delivery::lossless("FLAC", 16, 44_100))
                .with_edition("2000 original")
                .with_dynamic_range(13, "2000, CD, lossless"),
            purchase(Vendor::QobuzStore, Delivery::lossless("FLAC", 24, 96_000))
                .with_edition("2016 remaster")
                .with_dynamic_range(6, "2016, WEB, lossless"),
        ];
        let verdict = Verdict::assemble(None, rank(&offers, &Weights::default()), Vec::new());
        assert!(verdict
            .conflicts
            .iter()
            .any(|c| matches!(c, Conflict::RemasterIsFlatter { .. })));
        assert!(verdict.recommendation.contains("DR6"));
        assert!(verdict.recommendation.contains("DR13"));
    }

    #[test]
    fn multiple_masters_without_dr_still_warn_but_do_not_claim_one_is_worse() {
        let offers = vec![
            purchase(Vendor::Bandcamp, Delivery::lossless("FLAC", 16, 44_100))
                .with_edition("2000 original"),
            purchase(Vendor::QobuzStore, Delivery::lossless("FLAC", 24, 96_000))
                .with_edition("2016 remaster"),
        ];
        let verdict = Verdict::assemble(None, rank(&offers, &Weights::default()), Vec::new());
        assert!(verdict
            .conflicts
            .iter()
            .any(|c| matches!(c, Conflict::MultipleMasters { .. })));
        assert!(!verdict
            .conflicts
            .iter()
            .any(|c| matches!(c, Conflict::RemasterIsFlatter { .. })));
        assert!(verdict.recommendation.contains("more than one master"));
    }

    #[test]
    fn an_unset_key_is_not_reported_when_the_index_already_covers_that_shop() {
        // The complaint that produced this rule: "Not checked: Qobuz Store" under
        // every single ranking. Qobuz is in the list — MusicBrainz indexed a link
        // to it — so the token would only have added a price, and calling that a
        // gap that "may be missing a better option" is simply false.
        let offers = vec![Offer::new(
            Vendor::QobuzStore,
            Acquisition::Purchase,
            Delivery::lossless("FLAC", 16, 44_100),
        )
        .indexed()];
        let verdict = Verdict::assemble(
            None,
            rank(&offers, &Weights::default()),
            vec![(
                SourceId::QobuzStore,
                Reason::NotConfigured("no qobuz_user_token".into()),
            )],
        );
        assert!(verdict.unchecked.is_empty(), "{:?}", verdict.unchecked);
        assert!(!verdict.recommendation.contains("Not checked"));
    }

    #[test]
    fn an_unset_key_is_not_reported_even_when_nothing_stands_in_for_it() {
        // No Qobuz row from anywhere and still no mention. The person turned this
        // source off by leaving a key blank; telling them so under every album is
        // not new information, it is a reminder they cannot act on without
        // subscribing to something.
        let offers = vec![purchase(Vendor::ITunes, Delivery::lossy("AAC 256"))];
        let verdict = Verdict::assemble(
            None,
            rank(&offers, &Weights::default()),
            vec![(
                SourceId::QobuzStore,
                Reason::NotConfigured("no qobuz_user_token".into()),
            )],
        );
        assert!(verdict.unchecked.is_empty());
        assert!(!verdict.recommendation.contains("Not checked"));
    }

    #[test]
    fn a_source_that_actually_broke_is_reported_however_well_covered_it_is() {
        // Coverage excuses an unset key, never a failure. A rate-limited or
        // unreachable source might have had something the index does not.
        let offers = vec![Offer::new(
            Vendor::QobuzStore,
            Acquisition::Purchase,
            Delivery::lossless("FLAC", 16, 44_100),
        )
        .indexed()];
        let verdict = Verdict::assemble(
            None,
            rank(&offers, &Weights::default()),
            vec![(SourceId::QobuzStore, Reason::RateLimited)],
        );
        assert_eq!(verdict.unchecked.len(), 1);
    }

    #[test]
    fn a_source_that_answered_with_nothing_is_never_a_gap() {
        // The case behind "Not checked: MusicBrainz" on a 2002 album: the release
        // has link relations, none of them a shop, because nothing was sold as a
        // download then. That is an answer.
        let offers = vec![purchase(
            Vendor::Bandcamp,
            Delivery::lossless("FLAC", 16, 44_100),
        )];
        let verdict = Verdict::assemble(None, rank(&offers, &Weights::default()), Vec::new());
        assert!(verdict.unchecked.is_empty());
        assert!(!verdict.recommendation.contains("Not checked"));
    }

    #[test]
    fn an_incomplete_lookup_says_which_sources_were_missing() {
        let offers =
            vec![purchase(Vendor::ITunes, Delivery::lossy("AAC 256")).with_price(7.99, "GBP")];
        let verdict = Verdict::assemble(
            None,
            rank(&offers, &Weights::default()),
            vec![
                (SourceId::Bandcamp, Reason::Http(503)),
                (SourceId::QobuzStore, Reason::Network("no app id".into())),
            ],
        );
        assert!(verdict
            .recommendation
            .contains("Not checked: Bandcamp and Qobuz Store"));
        assert!(verdict.recommendation.contains("missing a better option"));
    }

    #[test]
    fn region_locked_offers_are_mentioned_but_not_recommended() {
        let offers = vec![
            purchase(Vendor::Bandcamp, Delivery::lossless("FLAC", 16, 44_100))
                .with_friction(Friction::RegionLocked),
            purchase(Vendor::ITunes, Delivery::lossy("AAC 256")).with_price(7.99, "GBP"),
        ];
        let verdict = Verdict::assemble(None, rank(&offers, &Weights::default()), Vec::new());
        assert!(verdict.recommendation.starts_with("iTunes Store"));
        assert!(verdict.recommendation.contains("not to your region"));
    }

    #[test]
    fn a_stream_only_album_says_the_catalogue_can_take_it_away() {
        let offers = vec![Offer::new(
            Vendor::Tidal,
            Acquisition::Subscription,
            Delivery::lossless("FLAC", 16, 44_100),
        )];
        let verdict = Verdict::assemble(None, rank(&offers, &Weights::default()), Vec::new());
        assert_eq!(verdict.tier(), Tier::C);
        assert!(verdict
            .recommendation
            .contains("No one sells this as files"));
        assert!(verdict.recommendation.contains("stays in the catalogue"));
    }

    #[test]
    fn lists_read_as_english() {
        assert_eq!(list(&[]), "");
        assert_eq!(list(&["a"]), "a");
        assert_eq!(list(&["a", "b"]), "a and b");
        assert_eq!(list(&["a", "b", "c"]), "a, b and c");
    }
}
