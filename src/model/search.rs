//! Fuzzy matching and disambiguation.
//!
//! MusicBrainz's own relevance score is the primary signal and a local text pass
//! re-ranks it, because neither is enough alone. MusicBrainz scores a Lucene
//! query, so it rewards field matches and knows nothing about a transposed
//! letter; the local pass knows about typos and knows nothing about which of two
//! identically titled records is the famous one. Blended, they cover each
//! other's blind spot.
//!
//! Everything here is a pure function over a parsed response. No I/O, no clock,
//! no display — the whole disambiguation story is exercised from fixtures.

use rapidfuzz::fuzz;

use super::album::ReleaseGroup;
use super::candidate::{Candidate, Matches, NearMiss, NearMissKind};
use super::query::{fold, Query};
use super::source::musicbrainz::ScoredGroup;

/// How much each signal is worth.
///
/// MusicBrainz and the title carry the weight; the artist is a third signal
/// rather than half the answer, because a search often has the artist slightly
/// wrong — that is the case this whole module exists to survive.
const WEIGHT_SEARCH: f64 = 0.4;
const WEIGHT_TITLE: f64 = 0.4;
const WEIGHT_ARTIST: f64 = 0.2;

/// Below this blended score, a result is not shown at all.
const FLOOR: f64 = 0.35;
/// A result whose title is further off than this is dropped whatever else says.
///
/// MusicBrainz's relevance is its own Lucene query's opinion, and it can score a
/// record highly for reasons that have nothing to do with what was typed. With
/// the search weight at 0.4, a strong enough MusicBrainz score clears the
/// blended floor on its own — so a plain textual disagreement about the title
/// has to be able to veto it.
const TITLE_VETO: f64 = 0.6;
/// A near miss must at least be recognisable.
const NEAR_MISS_FLOOR: f64 = 0.45;
/// Only a handful of each kind of near miss is useful.
const NEAR_MISS_LIMIT: usize = 5;

/// Rank search results and separate out the near misses.
pub fn rank_candidates(query: &Query, results: &[ScoredGroup]) -> Matches {
    let titled = !query.album.trim().is_empty();
    let mut candidates: Vec<Candidate> = results
        .iter()
        .map(|result| score(query, result))
        .filter(|candidate| candidate.blended >= FLOOR)
        .filter(|candidate| !titled || candidate.title_similarity >= TITLE_VETO)
        .collect();

    candidates.sort_by(|a, b| {
        b.blended
            .partial_cmp(&a.blended)
            .unwrap_or(std::cmp::Ordering::Equal)
            // Ties break on MBID so the list does not reshuffle between two
            // identical searches.
            .then_with(|| a.group.mbid.cmp(&b.group.mbid))
    });

    Matches {
        candidates,
        near_misses: Vec::new(),
    }
}

/// Add the near misses, from a browse of the leading artist's other records.
///
/// Kept as a second step because it needs a second request, and the confident
/// matches must render before it arrives rather than waiting on it.
pub fn add_near_misses(matches: &mut Matches, query: &Query, same_artist: &[ScoredGroup]) {
    let already: Vec<&str> = matches
        .candidates
        .iter()
        .map(|candidate| candidate.group.mbid.as_str())
        .collect();

    let mut misses: Vec<NearMiss> = same_artist
        .iter()
        .filter(|result| !already.contains(&result.group.mbid.as_str()))
        .map(|result| score(query, result))
        .filter(|candidate| candidate.title_similarity >= NEAR_MISS_FLOOR)
        .map(|candidate| NearMiss {
            candidate,
            kind: NearMissKind::SameArtistOtherRelease,
        })
        .collect();

    // Anything the main search turned up that scored well on title but poorly on
    // artist is the other kind of near miss: someone else's record with almost
    // this name. Covers records and soundtracks live here.
    misses.extend(
        matches
            .candidates
            .iter()
            .filter(|candidate| {
                candidate.title_similarity >= 0.75
                    && candidate.artist_similarity < 0.5
                    && !query.artist.is_empty()
            })
            .map(|candidate| NearMiss {
                candidate: candidate.clone(),
                kind: NearMissKind::OtherArtistSimilarTitle,
            }),
    );

    // Those are near misses, not answers — they leave the ranked list.
    matches
        .candidates
        .retain(|candidate| candidate.artist_similarity >= 0.5 || query.artist.is_empty());

    misses.sort_by(|a, b| {
        b.candidate
            .blended
            .partial_cmp(&a.candidate.blended)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.candidate.group.mbid.cmp(&b.candidate.group.mbid))
    });
    misses.dedup_by(|a, b| a.candidate.group.mbid == b.candidate.group.mbid);

    let mut same = 0;
    let mut other = 0;
    misses.retain(|miss| {
        let count = match miss.kind {
            NearMissKind::SameArtistOtherRelease => &mut same,
            NearMissKind::OtherArtistSimilarTitle => &mut other,
        };
        *count += 1;
        *count <= NEAR_MISS_LIMIT
    });

    matches.near_misses = misses;
}

fn score(query: &Query, result: &ScoredGroup) -> Candidate {
    let title_similarity = title_score(&query.album, &result.group.title);
    let artist_similarity = if query.artist.is_empty() {
        // Nothing was asked about the artist, so nothing is claimed about it.
        // Scoring it zero would drag every candidate below the floor.
        1.0
    } else {
        artist_score(&query.artist, &result.group)
    };
    let search_score = f64::from(result.score) / 100.0;

    Candidate {
        group: result.group.clone(),
        search_score,
        title_similarity,
        artist_similarity,
        blended: WEIGHT_SEARCH * search_score
            + WEIGHT_TITLE * title_similarity
            + WEIGHT_ARTIST * artist_similarity,
    }
}

/// Compare two album titles.
///
/// Scored twice and the better taken: once on the query as typed, and once with
/// any bracketed edition stripped from both sides. That is what makes a search
/// for "Kid A" match "Kid A (Collector's Edition)" at full strength while still
/// letting someone who typed the edition out get a better score for the one they
/// named.
fn title_score(query: &str, title: &str) -> f64 {
    let direct = similarity(&fold(query), &fold(title));
    let (query_base, _) = super::query::strip_edition(query);
    let (title_base, _) = super::query::strip_edition(title);
    let stripped = similarity(&fold(&query_base), &fold(&title_base));
    direct.max(stripped)
}

/// Compare a typed artist against every name the credited artist goes by.
fn artist_score(query: &str, group: &ReleaseGroup) -> f64 {
    let folded = fold(query);
    group
        .artist
        .every_name()
        .into_iter()
        .map(|name| similarity(&folded, &fold(name)))
        .fold(0.0, f64::max)
}

/// Half token-set, half plain edit distance.
///
/// Neither alone is usable. `token_set_ratio` is by construction blind to extra
/// words — when one side's tokens are a subset of the other's it compares the
/// shared tokens against themselves and returns a perfect match, so "Kid A"
/// scores 1.0 against "Kid A Mnesia" and the two become indistinguishable. A
/// plain ratio is the opposite: it punishes "Symphony 5" against "Symphony No. 5
/// in C Minor, Op. 67" into oblivion.
///
/// Blending them keeps the tolerance for a missing subtitle while making extra
/// words cost something, which is what separates an album from its companion
/// release. This is what the Python library's `WRatio` is for; the Rust port
/// ships neither it nor `token_set_ratio`, so both are built here.
fn similarity(a: &str, b: &str) -> f64 {
    0.5 * token_set_ratio(a, b) + 0.5 * plain_ratio(a, b)
}

fn plain_ratio(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return if a == b { 1.0 } else { 0.0 };
    }
    fuzz::ratio(a.chars(), b.chars())
}

/// Order-insensitive similarity that tolerates extra words on either side.
///
/// The Rust port of rapidfuzz ships only `ratio`, so this is the standard token
/// set construction on top of it: compare the shared words against each side's
/// shared-plus-unique words, and take the best of the three. It is what makes
/// "Beethoven Symphony 5" match "Symphony No. 5 in C Minor — Beethoven", where a
/// plain edit distance scores near zero.
fn token_set_ratio(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return if a == b { 1.0 } else { 0.0 };
    }

    let mut left: Vec<&str> = a.split_whitespace().collect();
    let mut right: Vec<&str> = b.split_whitespace().collect();
    left.sort_unstable();
    left.dedup();
    right.sort_unstable();
    right.dedup();

    let shared: Vec<&str> = left
        .iter()
        .filter(|word| right.contains(word))
        .copied()
        .collect();
    let only_left: Vec<&str> = left
        .iter()
        .filter(|word| !right.contains(word))
        .copied()
        .collect();
    let only_right: Vec<&str> = right
        .iter()
        .filter(|word| !left.contains(word))
        .copied()
        .collect();

    let base = shared.join(" ");
    let with_left = [shared.clone(), only_left].concat().join(" ");
    let with_right = [shared, only_right].concat().join(" ");

    // The Rust port returns 0.0–1.0 here, where the Python library it is a port
    // of returns 0–100. Scaling this by a hundredth was the first bug in this
    // file, and it made every title look like a non-match.
    let ratio = |x: &str, y: &str| fuzz::ratio(x.chars(), y.chars());
    ratio(&base, &with_left)
        .max(ratio(&base, &with_right))
        .max(ratio(&with_left, &with_right))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::album::{ArtistCredit, Mbid};

    fn group(title: &str, artist: &str, score: u8) -> ScoredGroup {
        ScoredGroup {
            group: ReleaseGroup {
                mbid: Mbid::new(format!("{artist}-{title}")),
                title: title.into(),
                artist: ArtistCredit::new(artist),
                ..ReleaseGroup::default()
            },
            score,
        }
    }

    #[test]
    fn an_exact_match_beats_everything_around_it() {
        let query = Query::new("Radiohead", "Kid A");
        let matches = rank_candidates(
            &query,
            &[
                group("Kid A Mnesia", "Radiohead", 80),
                group("Kid A", "Radiohead", 100),
                group("Amnesiac", "Radiohead", 40),
            ],
        );
        assert_eq!(matches.best().unwrap().group.title, "Kid A");
    }

    #[test]
    fn a_typo_in_the_artist_still_finds_the_album() {
        // The case the local pass exists for: MusicBrainz scores this poorly
        // because the Lucene artist clause misses, and the text pass rescues it.
        let query = Query::new("Radiohed", "Kid A");
        let matches = rank_candidates(&query, &[group("Kid A", "Radiohead", 45)]);
        assert!(matches.best().unwrap().artist_similarity > 0.8);
        assert!(matches.best().unwrap().blended >= FLOOR);
    }

    #[test]
    fn a_missing_subtitle_matches_at_full_strength() {
        // Someone types the album; the catalogue has the deluxe edition. The
        // bracketed part is not a difference in the name.
        let query = Query::new("Radiohead", "Kid A");
        let matches = rank_candidates(
            &query,
            &[group("Kid A (Collector's Edition)", "Radiohead", 90)],
        );
        assert!(
            matches.best().unwrap().title_similarity > 0.95,
            "similarity was {}",
            matches.best().unwrap().title_similarity
        );
    }

    #[test]
    fn word_order_and_extra_words_do_not_sink_a_classical_title() {
        let query = Query::new("Beethoven", "Symphony 5");
        let matches = rank_candidates(
            &query,
            &[group(
                "Symphony No. 5 in C Minor, Op. 67",
                "Ludwig van Beethoven",
                70,
            )],
        );
        assert!(
            !matches.candidates.is_empty(),
            "the symphony was filtered out"
        );
    }

    #[test]
    fn an_accented_artist_matches_the_unaccented_spelling() {
        let query = Query::new("Bjork", "Homogenic");
        let matches = rank_candidates(&query, &[group("Homogenic", "Björk", 90)]);
        assert!(matches.best().unwrap().artist_similarity > 0.95);
    }

    #[test]
    fn a_native_script_name_matches_through_the_alias_list() {
        let mut scored = group("async", "Ryuichi Sakamoto", 80);
        scored.group.artist.aliases = vec!["坂本龍一".into()];
        let matches = rank_candidates(&Query::new("坂本龍一", "async"), &[scored]);
        assert!(matches.best().unwrap().artist_similarity > 0.95);
    }

    #[test]
    fn searching_without_an_artist_does_not_penalise_every_result() {
        let matches = rank_candidates(&Query::new("", "Kid A"), &[group("Kid A", "Radiohead", 95)]);
        assert_eq!(matches.best().unwrap().artist_similarity, 1.0);
        assert!(matches.best().unwrap().blended > 0.9);
    }

    #[test]
    fn the_same_search_twice_produces_the_same_order() {
        let query = Query::new("Various", "Greatest Hits");
        let results = [
            group("Greatest Hits", "Band A", 90),
            group("Greatest Hits", "Band B", 90),
        ];
        let first = rank_candidates(&query, &results);
        let mut reversed = results.to_vec();
        reversed.reverse();
        assert_eq!(first, rank_candidates(&query, &reversed));
    }

    #[test]
    fn another_artists_similarly_titled_record_becomes_a_near_miss_not_a_candidate() {
        // A search for Radiohead's Kid A must not lead with a covers band's.
        let query = Query::new("Radiohead", "Kid A");
        let mut matches = rank_candidates(
            &query,
            &[
                group("Kid A", "Radiohead", 100),
                group("Kid A", "The String Quartet Tribute", 85),
            ],
        );
        add_near_misses(&mut matches, &query, &[]);

        assert_eq!(matches.candidates.len(), 1);
        assert_eq!(matches.candidates[0].group.artist.name, "Radiohead");
        assert!(matches.near_misses.iter().any(|miss| {
            miss.kind == NearMissKind::OtherArtistSimilarTitle
                && miss.candidate.group.artist.name == "The String Quartet Tribute"
        }));
    }

    #[test]
    fn the_same_artists_other_records_are_offered_separately() {
        let query = Query::new("Radiohead", "Kid A");
        let mut matches = rank_candidates(&query, &[group("Kid A", "Radiohead", 100)]);
        add_near_misses(
            &mut matches,
            &query,
            &[
                group("Kid A Mnesia", "Radiohead", 0),
                group("Amnesiac", "Radiohead", 0),
            ],
        );

        // Kid A Mnesia is recognisably close; Amnesiac is not, and showing every
        // record an artist made would bury the answer.
        let same: Vec<&str> = matches
            .near_misses
            .iter()
            .filter(|miss| miss.kind == NearMissKind::SameArtistOtherRelease)
            .map(|miss| miss.candidate.group.title.as_str())
            .collect();
        assert!(same.contains(&"Kid A Mnesia"));
        assert!(!same.contains(&"Amnesiac"));
    }

    #[test]
    fn a_candidate_already_in_the_ranked_list_is_not_repeated_as_a_near_miss() {
        let query = Query::new("Radiohead", "Kid A");
        let mut matches = rank_candidates(&query, &[group("Kid A", "Radiohead", 100)]);
        add_near_misses(&mut matches, &query, &[group("Kid A", "Radiohead", 0)]);
        assert!(matches.near_misses.is_empty());
    }

    #[test]
    fn near_misses_are_capped_so_they_cannot_bury_the_answer() {
        let query = Query::new("Various", "Hits");
        let mut matches = rank_candidates(&query, &[group("Hits", "Various", 100)]);
        let others: Vec<ScoredGroup> = (0..20)
            .map(|n| group(&format!("Hits Volume {n}"), "Various", 0))
            .collect();
        add_near_misses(&mut matches, &query, &others);
        assert!(matches.near_misses.len() <= NEAR_MISS_LIMIT);
    }

    #[test]
    fn nonsense_finds_nothing_rather_than_the_least_bad_thing() {
        let matches = rank_candidates(
            &Query::new("zzzzqqq", "xxxxwwww"),
            &[group("Kid A", "Radiohead", 5)],
        );
        assert!(matches.candidates.is_empty());
    }

    #[test]
    fn token_set_ratio_handles_the_shapes_it_was_added_for() {
        // Reordered words.
        assert!(token_set_ratio("kind of blue", "blue of kind") > 0.95);
        // A superset on one side.
        assert!(token_set_ratio("kid a", "kid a collector s edition") > 0.95);
        // Genuinely different.
        assert!(token_set_ratio("kid a", "the bends") < 0.5);
        // Degenerate inputs do not panic or claim a match.
        assert_eq!(token_set_ratio("", ""), 1.0);
        assert_eq!(token_set_ratio("a", ""), 0.0);
    }
}
