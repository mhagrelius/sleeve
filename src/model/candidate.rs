//! What a search comes back with, and how sure it is.
//!
//! Candidates are never auto-picked. Album and artist names collide constantly —
//! self-titled records, soundtracks, and any composer with fifteen editions of
//! one score — and a tool that quietly chooses for you in those cases is a tool
//! that quietly recommends the wrong pressing.

use crate::model::album::ReleaseGroup;

/// How well a release group answers the query.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub group: ReleaseGroup,
    /// MusicBrainz's own relevance, 0.0–1.0.
    pub search_score: f64,
    pub title_similarity: f64,
    pub artist_similarity: f64,
    /// The blend the list is ordered by, 0.0–1.0.
    pub blended: f64,
}

/// Whether the top answer is good enough to lead with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// One candidate is clearly the answer.
    Confident,
    /// Several plausible answers, or one weak one. Show them and ask.
    Ambiguous,
}

/// Why a near miss is being shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NearMissKind {
    /// Same artist, a differently named record.
    ///
    /// The most useful kind: it is how a person finds the edition, live album or
    /// reissue they actually meant.
    SameArtistOtherRelease,
    /// A different artist with a very similar album title.
    ///
    /// Soundtracks and classical live here, along with every covers record.
    OtherArtistSimilarTitle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NearMiss {
    pub candidate: Candidate,
    pub kind: NearMissKind,
}

/// Everything a search produced.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Matches {
    /// Ranked, best first.
    pub candidates: Vec<Candidate>,
    /// Kept apart from the ranked list, never interleaved.
    pub near_misses: Vec<NearMiss>,
}

impl Matches {
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty() && self.near_misses.is_empty()
    }

    pub fn best(&self) -> Option<&Candidate> {
        self.candidates.first()
    }

    /// Whether the top candidate can be led with.
    ///
    /// Two conditions, and both are needed. A high score alone is not enough:
    /// fifteen editions of the same symphony all score highly against a search
    /// for that symphony, and picking the first is a coin toss dressed up as an
    /// answer. So the leader must also be clearly ahead of the runner-up.
    pub fn confidence(&self) -> Confidence {
        let Some(best) = self.best() else {
            return Confidence::Ambiguous;
        };
        if best.blended < CONFIDENT_THRESHOLD {
            return Confidence::Ambiguous;
        }
        let runner_up = self.candidates.get(1).map(|c| c.blended).unwrap_or(0.0);
        if best.blended - runner_up < CLEAR_MARGIN {
            return Confidence::Ambiguous;
        }
        Confidence::Confident
    }
}

/// How good the leader must be, on its own.
pub const CONFIDENT_THRESHOLD: f64 = 0.80;
/// How far ahead of the runner-up the leader must be.
pub const CLEAR_MARGIN: f64 = 0.10;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::album::{ArtistCredit, Mbid};

    fn candidate(title: &str, blended: f64) -> Candidate {
        Candidate {
            group: ReleaseGroup {
                mbid: Mbid::new(title),
                title: title.into(),
                artist: ArtistCredit::new("Someone"),
                ..ReleaseGroup::default()
            },
            search_score: blended,
            title_similarity: blended,
            artist_similarity: blended,
            blended,
        }
    }

    #[test]
    fn one_clear_leader_is_confident() {
        let matches = Matches {
            candidates: vec![candidate("Kid A", 0.97), candidate("Kid A Mnesia", 0.62)],
            near_misses: Vec::new(),
        };
        assert_eq!(matches.confidence(), Confidence::Confident);
    }

    #[test]
    fn two_equally_good_answers_are_ambiguous_however_high_they_score() {
        // Fifteen editions of one score all match it perfectly. Leading with
        // whichever sorted first would be a guess presented as a result.
        let matches = Matches {
            candidates: vec![
                candidate("Symphony No. 5", 0.98),
                candidate("Symphony No. 5", 0.96),
            ],
            near_misses: Vec::new(),
        };
        assert_eq!(matches.confidence(), Confidence::Ambiguous);
    }

    #[test]
    fn a_weak_leader_is_ambiguous_even_with_nothing_behind_it() {
        let matches = Matches {
            candidates: vec![candidate("Something Else", 0.55)],
            near_misses: Vec::new(),
        };
        assert_eq!(matches.confidence(), Confidence::Ambiguous);
    }

    #[test]
    fn a_strong_lone_result_is_confident() {
        let matches = Matches {
            candidates: vec![candidate("Kid A", 0.94)],
            near_misses: Vec::new(),
        };
        assert_eq!(matches.confidence(), Confidence::Confident);
    }

    #[test]
    fn nothing_found_is_ambiguous_rather_than_a_panic() {
        assert_eq!(Matches::default().confidence(), Confidence::Ambiguous);
        assert!(Matches::default().is_empty());
    }
}
