//! How hard each source may be asked.
//!
//! The budgets differ by two orders of magnitude and two of them are not
//! advertised, so this is a table rather than a constant. `ui::http` reads it to
//! size a token bucket per source; nothing here does any timing itself, which is
//! what keeps the policy assertable in a unit test.

use std::time::Duration;

use super::SourceId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Smallest gap between two requests to this source.
    pub min_interval: Duration,
    /// How long to wait for one response before giving up on it.
    pub timeout: Duration,
    /// How long to wait after being told to slow down.
    pub backoff: Duration,
    /// Whether a failure here should stop the lookup.
    ///
    /// Only MusicBrainz is required: without an identity there is nothing to
    /// look up. Everything else degrades to a missing row.
    pub required: bool,
}

/// The budget for one source.
pub fn for_source(source: SourceId) -> Policy {
    match source {
        // A hard, published, enforced limit. Exceed it and they block the
        // User-Agent, not just the request.
        SourceId::MusicBrainz => Policy {
            min_interval: Duration::from_millis(1000),
            timeout: Duration::from_secs(10),
            backoff: Duration::from_secs(5),
            required: true,
        },
        // Roughly ten a minute unauthenticated, which is the tightest budget of
        // the lot. Only ever called once a release is chosen, never during a
        // search — six seconds apart keeps a drill-down session inside it.
        SourceId::Odesli => Policy {
            min_interval: Duration::from_millis(6000),
            timeout: Duration::from_secs(10),
            backoff: Duration::from_secs(30),
            required: false,
        },
        // About twenty a minute, uncredentialed.
        SourceId::ITunes => Policy {
            min_interval: Duration::from_millis(3000),
            timeout: Duration::from_secs(8),
            backoff: Duration::from_secs(20),
            required: false,
        },
        // Sixty a minute with a token.
        SourceId::Discogs => Policy {
            min_interval: Duration::from_millis(1000),
            timeout: Duration::from_secs(10),
            backoff: Duration::from_secs(10),
            required: false,
        },
        // Undocumented, so unmeasured. Slower than we need rather than as fast
        // as they will tolerate.
        SourceId::Bandcamp | SourceId::QobuzStore => Policy {
            min_interval: Duration::from_millis(1500),
            timeout: Duration::from_secs(10),
            backoff: Duration::from_secs(60),
            required: false,
        },
        // Returned HTTP 429 on the very first request during development, with
        // both a polite User-Agent and a browser one. Treated as hostile: one
        // request every five seconds, a long backoff, and never on the critical
        // path of a lookup.
        SourceId::DynamicRange => Policy {
            min_interval: Duration::from_millis(5000),
            timeout: Duration::from_secs(15),
            backoff: Duration::from_secs(300),
            required: false,
        },
        // Images. Slow and 404-heavy by design, and never blocking.
        SourceId::CoverArtArchive => Policy {
            min_interval: Duration::from_millis(200),
            timeout: Duration::from_secs(6),
            backoff: Duration::from_secs(30),
            required: false,
        },
        SourceId::Deezer => Policy {
            min_interval: Duration::from_millis(500),
            timeout: Duration::from_secs(8),
            backoff: Duration::from_secs(30),
            required: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn musicbrainz_is_never_asked_faster_than_once_a_second() {
        // Their published limit, and the one whose breach gets an application
        // blocked rather than throttled.
        assert!(for_source(SourceId::MusicBrainz).min_interval >= Duration::from_secs(1));
    }

    #[test]
    fn only_musicbrainz_can_stop_a_lookup() {
        for source in SourceId::ALL {
            assert_eq!(
                for_source(source).required,
                source == SourceId::MusicBrainz,
                "{source} required flag"
            );
        }
    }

    #[test]
    fn the_dynamic_range_database_backs_off_longest_of_all() {
        // It rate-limited us on request one, with a polite User-Agent and with a
        // browser one. Whatever else changes in this table, a 429 from it must
        // cost more than a 429 from anywhere else.
        let dr = for_source(SourceId::DynamicRange);
        for source in SourceId::ALL {
            assert!(dr.backoff >= for_source(source).backoff, "{source}");
        }
    }

    #[test]
    fn the_two_sources_with_the_tightest_published_budgets_are_polled_slowest() {
        // Odesli allows about ten requests a minute and the Dynamic Range DB
        // tolerates less than it says; everything else is faster than both.
        let slow = [SourceId::Odesli, SourceId::DynamicRange];
        let fastest_slow = slow
            .iter()
            .map(|source| for_source(*source).min_interval)
            .min()
            .unwrap();
        for source in SourceId::ALL {
            if slow.contains(&source) {
                continue;
            }
            assert!(
                for_source(source).min_interval < fastest_slow,
                "{source} is polled no faster than the throttled sources"
            );
        }
    }

    #[test]
    fn every_source_has_a_finite_timeout() {
        // A source with no timeout is a source that can hang the view forever.
        for source in SourceId::ALL {
            let policy = for_source(source);
            assert!(policy.timeout > Duration::ZERO, "{source}");
            assert!(policy.timeout <= Duration::from_secs(30), "{source}");
        }
    }
}
