//! Disambiguation, against a real MusicBrainz response.
//!
//! The recorded search for "Radiohead / Kid A" is a good adversary: twenty
//! results, five of them scoring above 80, and four of those are not the album —
//! *KID A MNESIA*, *The Kid A Theory*, *Kid 17*, *Kid Alive*. Anything that
//! leads with the wrong one of those is broken in a way a synthetic fixture
//! would not have shown.

use sleeve::model::candidate::{Confidence, NearMissKind};
use sleeve::model::query::Query;
use sleeve::model::search::{add_near_misses, rank_candidates};
use sleeve::model::source::{musicbrainz, Outcome};

fn recorded() -> Vec<musicbrainz::ScoredGroup> {
    let path = format!(
        "{}/tests/fixtures/musicbrainz/search-kid-a.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let body = std::fs::read(path).expect("the fixture");
    match musicbrainz::parse_release_groups(&body) {
        Outcome::Found(groups) => groups,
        other => panic!("expected groups, got {other:?}"),
    }
}

#[test]
fn the_exact_album_leads_a_field_of_near_identical_titles() {
    let matches = rank_candidates(&Query::new("Radiohead", "Kid A"), &recorded());
    let best = matches.best().expect("a best candidate");
    assert_eq!(best.group.title, "Kid A");
    assert_eq!(best.group.artist.name, "Radiohead");
    assert_eq!(matches.confidence(), Confidence::Confident);
}

#[test]
fn a_typo_in_the_artist_still_leads_with_the_album() {
    // MusicBrainz's own score drops when the Lucene artist clause misses; the
    // local pass is what keeps the answer on top.
    let matches = rank_candidates(&Query::new("Radiohed", "Kid A"), &recorded());
    assert_eq!(matches.best().unwrap().group.title, "Kid A");
}

#[test]
fn searching_for_the_companion_release_finds_that_one_instead() {
    // The two are one word apart and both by Radiohead, which is exactly the
    // case where auto-picking the top MusicBrainz hit would be wrong.
    let matches = rank_candidates(&Query::new("Radiohead", "Kid A Mnesia"), &recorded());
    assert_eq!(matches.best().unwrap().group.title, "KID A MNESIA");
}

#[test]
fn the_artists_other_records_arrive_as_near_misses_not_as_answers() {
    let query = Query::new("Radiohead", "Kid A");
    let mut matches = rank_candidates(&query, &recorded());
    let before = matches.candidates.len();

    add_near_misses(&mut matches, &query, &recorded());

    // Nothing already ranked is repeated below.
    let ranked: Vec<&str> = matches
        .candidates
        .iter()
        .map(|candidate| candidate.group.mbid.as_str())
        .collect();
    for miss in &matches.near_misses {
        assert!(
            !ranked.contains(&miss.candidate.group.mbid.as_str()),
            "{} appears twice",
            miss.candidate.group.title
        );
    }
    assert!(before >= matches.candidates.len());
}

#[test]
fn near_misses_are_capped_however_many_the_artist_has() {
    let query = Query::new("Radiohead", "Kid A");
    let mut matches = rank_candidates(&query, &recorded());
    add_near_misses(&mut matches, &query, &recorded());

    for kind in [
        NearMissKind::SameArtistOtherRelease,
        NearMissKind::OtherArtistSimilarTitle,
    ] {
        let count = matches
            .near_misses
            .iter()
            .filter(|miss| miss.kind == kind)
            .count();
        assert!(count <= 5, "{count} near misses of one kind");
    }
}

#[test]
fn an_album_search_with_no_artist_still_finds_it() {
    let matches = rank_candidates(&Query::new("", "Kid A"), &recorded());
    assert_eq!(matches.best().unwrap().group.title, "Kid A");
}

#[test]
fn a_search_for_something_absent_returns_nothing_rather_than_the_nearest_thing() {
    // Twenty Radiohead records and none of them is this one. Scoring the wrong
    // query against a recorded response is artificial — MusicBrainz would not
    // have returned these — but it pins the property that matters: a strong
    // relevance score must not be able to carry a record whose title plainly
    // disagrees. Without the title veto, "Kid Alive" comes back for this.
    let matches = rank_candidates(&Query::new("Miles Davis", "Kind of Blue"), &recorded());
    assert!(
        matches.candidates.is_empty(),
        "matched {:?}",
        matches.best().map(|c| &c.group.title)
    );
}

#[test]
fn every_candidate_carries_enough_to_tell_it_from_its_neighbours() {
    // The list is the disambiguation. A row with no year and no type is a row a
    // person cannot choose between.
    let matches = rank_candidates(&Query::new("Radiohead", "Kid A"), &recorded());
    for candidate in matches.candidates.iter().take(5) {
        let subtitle = candidate.group.subtitle();
        assert!(
            !subtitle.is_empty(),
            "{} has nothing to tell it apart",
            candidate.group.title
        );
        assert!(!candidate.group.artist.name.is_empty());
        assert!(!candidate.group.mbid.as_str().is_empty());
    }
}

#[test]
fn the_same_search_run_twice_gives_the_same_list() {
    let query = Query::new("Radiohead", "Kid A");
    assert_eq!(
        rank_candidates(&query, &recorded()),
        rank_candidates(&query, &recorded())
    );
}
