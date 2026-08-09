//! The seam between "what to ask" and "asking it".
//!
//! Every source in this directory is a pair of pure functions: one builds a
//! [`Request`], the other turns a response body into an [`Outcome`]. Nothing
//! here opens a socket — `ui::http` does that, and it is the only file in the
//! tree that does. That is what makes every source, every malformed response and
//! every failure mode testable from a recorded fixture with no network and no
//! display.
//!
//! There is deliberately no `Source` trait. The eight sources answer genuinely
//! different questions — MusicBrainz answers "what is this record", Discogs
//! answers "what pressings exist", Bandcamp answers "will they sell it to me" —
//! and a trait returning one uniform type would be a fiction that costs a
//! conversion at both ends. What they share is the request/parse shape above,
//! and that is expressed by the two types rather than by a supertype.

pub mod art;
pub mod bandcamp;
pub mod discogs;
pub mod dynamic_range;
pub mod itunes;
pub mod musicbrainz;
pub mod odesli;
pub mod policy;
pub mod qobuz;

use std::fmt;

/// Who is being asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceId {
    MusicBrainz,
    Odesli,
    ITunes,
    Discogs,
    Bandcamp,
    QobuzStore,
    DynamicRange,
    CoverArtArchive,
    Deezer,
}

impl SourceId {
    pub const ALL: [SourceId; 9] = [
        SourceId::MusicBrainz,
        SourceId::Odesli,
        SourceId::ITunes,
        SourceId::Discogs,
        SourceId::Bandcamp,
        SourceId::QobuzStore,
        SourceId::DynamicRange,
        SourceId::CoverArtArchive,
        SourceId::Deezer,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SourceId::MusicBrainz => "MusicBrainz",
            SourceId::Odesli => "Odesli",
            SourceId::ITunes => "iTunes",
            SourceId::Discogs => "Discogs",
            SourceId::Bandcamp => "Bandcamp",
            SourceId::QobuzStore => "Qobuz Store",
            SourceId::DynamicRange => "Dynamic Range DB",
            SourceId::CoverArtArchive => "Cover Art Archive",
            SourceId::Deezer => "Deezer",
        }
    }

    /// The shops this source can put in a ranking.
    ///
    /// Used to decide whether a source being unconfigured is worth mentioning:
    /// if every shop it could have supplied is already in the list — because
    /// MusicBrainz indexed a link to it — then nothing is missing and saying
    /// "not checked" is noise on every single lookup.
    pub fn vendors(self) -> &'static [crate::model::offer::Vendor] {
        use crate::model::offer::Vendor::*;
        match self {
            SourceId::Bandcamp => &[Bandcamp],
            SourceId::QobuzStore => &[QobuzStore, Qobuz],
            SourceId::ITunes => &[ITunes],
            SourceId::Discogs => &[PhysicalUsed, PhysicalNew],
            SourceId::Odesli => &[
                Spotify,
                AppleMusic,
                Tidal,
                Deezer,
                AmazonMusicHd,
                YouTubeMusic,
            ],
            // These supply no shop at all. MusicBrainz is identity and the index
            // itself; the others decorate. Their absence is always worth saying.
            SourceId::MusicBrainz
            | SourceId::DynamicRange
            | SourceId::CoverArtArchive
            | SourceId::Deezer => &[],
        }
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// One HTTP call, described but not made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub source: SourceId,
    pub url: String,
    pub headers: Vec<(String, String)>,
}

impl Request {
    pub fn get(source: SourceId, url: impl Into<String>) -> Self {
        Request {
            source,
            url: url.into(),
            headers: Vec::new(),
        }
    }

    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }

    /// The cache key for this request.
    ///
    /// The URL is the key: two requests that differ only in a header are, for
    /// every source here, the same question.
    pub fn cache_key(&self) -> &str {
        &self.url
    }
}

/// Why a source could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    Timeout,
    RateLimited,
    Http(u16),
    /// The body arrived and did not parse.
    Malformed(String),
    /// Never asked: no API key, no app id, or switched off.
    NotConfigured(String),
    Network(String),
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::Timeout => write!(f, "timed out"),
            Reason::RateLimited => write!(f, "rate limited"),
            Reason::Http(code) => write!(f, "HTTP {code}"),
            Reason::Malformed(what) => write!(f, "unreadable response: {what}"),
            Reason::NotConfigured(what) => write!(f, "not configured: {what}"),
            Reason::Network(what) => write!(f, "{what}"),
        }
    }
}

/// What a source came back with.
///
/// The distinction between [`Outcome::Empty`] and [`Outcome::Stale`] is the
/// whole reason this is not an `Option` or a `Result`. Half of these sources are
/// undocumented endpoints and one is an HTML table; those do not fail with a
/// status code, they answer `200 OK` with a page that no longer contains what we
/// parse. If "this shop does not stock it" and "our parser broke" are the same
/// value, a broken Bandcamp reader silently deletes the winner from every
/// ranking and the result still looks like a confident, complete answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome<T> {
    /// The source answered and had something.
    Found(T),
    /// The source answered and genuinely has nothing.
    Empty,
    /// The source could not be reached or refused.
    Unusable(Reason),
    /// The source answered, but not in a shape we recognise.
    ///
    /// Rendered to the person as "check failed" rather than silence, and it is
    /// what the fixture tests assert on when a recorded body is refreshed.
    Stale(Reason),
}

impl<T> Outcome<T> {
    pub fn found(self) -> Option<T> {
        match self {
            Outcome::Found(value) => Some(value),
            _ => None,
        }
    }

    pub fn is_found(&self) -> bool {
        matches!(self, Outcome::Found(_))
    }

    /// Why this source contributed nothing, if it should be reported as a gap.
    ///
    /// [`Outcome::Empty`] returns `None`: a source that answered and had nothing
    /// is not a gap in the lookup, it is an answer.
    ///
    /// The [`Reason`] comes back whole rather than as a formatted string, because
    /// the verdict has to tell [`Reason::NotConfigured`] — "you never asked for
    /// this" — from a source that actually broke. Only the second is a warning.
    pub fn gap(&self) -> Option<Reason> {
        match self {
            Outcome::Found(_) | Outcome::Empty => None,
            Outcome::Unusable(reason) => Some(reason.clone()),
            Outcome::Stale(reason) => Some(Reason::Malformed(format!("check failed — {reason}"))),
        }
    }

    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Outcome<U> {
        match self {
            Outcome::Found(value) => Outcome::Found(f(value)),
            Outcome::Empty => Outcome::Empty,
            Outcome::Unusable(reason) => Outcome::Unusable(reason),
            Outcome::Stale(reason) => Outcome::Stale(reason),
        }
    }

    /// `Found` when the collection has anything in it, `Empty` when it does not.
    pub fn of_collection(items: Vec<T>) -> Outcome<Vec<T>> {
        if items.is_empty() {
            Outcome::Empty
        } else {
            Outcome::Found(items)
        }
    }
}

/// Parse a body as JSON, mapping a parse failure to [`Outcome::Stale`].
///
/// A source that answers `200` with something that is not JSON has changed shape
/// under us; that is the definition of stale, and it is never `Empty`.
pub fn parse_json<T>(source: SourceId, body: &[u8]) -> Result<serde_json::Value, Outcome<T>> {
    serde_json::from_slice(body).map_err(|error| {
        Outcome::Stale(Reason::Malformed(format!(
            "{source} sent non-JSON: {error}"
        )))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_an_answer_and_stale_is_a_gap() {
        // The distinction the whole enum exists for.
        assert_eq!(Outcome::<u8>::Empty.gap(), None);
        assert_eq!(Outcome::Found(1u8).gap(), None);
        assert!(Outcome::<u8>::Unusable(Reason::Timeout)
            .gap()
            .is_some_and(|gap| gap.to_string().contains("timed out")));
        assert!(Outcome::<u8>::Stale(Reason::Malformed("no table".into()))
            .gap()
            .is_some_and(|gap| gap.to_string().contains("check failed")));
    }

    #[test]
    fn only_the_sources_that_supply_a_shop_can_be_covered_by_the_index() {
        use crate::model::offer::Vendor;

        // Qobuz without a token is the case this exists for: MusicBrainz already
        // says whether Qobuz sells the album, so an unset token costs nothing and
        // must not be reported on every lookup.
        assert!(SourceId::QobuzStore.vendors().contains(&Vendor::QobuzStore));
        assert!(SourceId::QobuzStore.vendors().contains(&Vendor::Qobuz));

        // The dynamic-range database supplies no shop at all, so nothing can ever
        // stand in for it and its absence is always worth saying.
        assert!(SourceId::DynamicRange.vendors().is_empty());
        assert!(SourceId::MusicBrainz.vendors().is_empty());
    }

    #[test]
    fn a_non_json_body_is_stale_rather_than_empty() {
        // Cloudflare's block page is valid HTML and a 200. Treating it as "no
        // results" would quietly drop a source from every ranking.
        let outcome: Outcome<u8> = match parse_json(SourceId::Bandcamp, b"<html>nope</html>") {
            Err(outcome) => outcome,
            Ok(_) => panic!("html parsed as json"),
        };
        assert!(matches!(outcome, Outcome::Stale(_)));
    }

    #[test]
    fn a_collection_of_nothing_is_empty() {
        assert_eq!(Outcome::of_collection(Vec::<u8>::new()), Outcome::Empty);
        assert_eq!(Outcome::of_collection(vec![1u8]), Outcome::Found(vec![1]));
    }

    #[test]
    fn a_request_keys_its_cache_entry_on_the_url_not_the_headers() {
        let a = Request::get(SourceId::Discogs, "https://example/x").header("Authorization", "one");
        let b = Request::get(SourceId::Discogs, "https://example/x").header("Authorization", "two");
        assert_eq!(a.cache_key(), b.cache_key());
    }
}
