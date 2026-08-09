//! Musical identity, as MusicBrainz models it.
//!
//! The distinction that matters throughout: a **release group** is the album as
//! a work — *Kid A* — and a **release** is one issue of it — the 2000 UK CD, the
//! 2009 remaster, the 2021 vinyl reissue. They are kept apart everywhere here.
//! Flattening them is how you end up recommending a shop that sells a different
//! master from the one that was asked about, and "which pressing" is the whole
//! question this application exists to answer.

use std::fmt;

/// A MusicBrainz identifier.
///
/// A newtype rather than a `String` because release ids and release-group ids
/// are both 36-character UUIDs, are not interchangeable, and are passed to
/// different endpoints.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mbid(String);

impl Mbid {
    pub fn new(id: impl Into<String>) -> Self {
        Mbid(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Mbid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Who made it, with every name they are known by.
///
/// `aliases` carries stage names, legal names, transliterations and
/// native-script forms. Matching runs against all of them, which is what lets a
/// search for "Beyonce" find "Beyoncé", and one for "Sakamoto Ryuichi" find
/// "坂本龍一".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtistCredit {
    pub name: String,
    pub mbid: Option<Mbid>,
    pub sort_name: Option<String>,
    pub aliases: Vec<String>,
}

impl ArtistCredit {
    pub fn new(name: impl Into<String>) -> Self {
        ArtistCredit {
            name: name.into(),
            ..ArtistCredit::default()
        }
    }

    /// Every string this artist could reasonably be typed as.
    pub fn every_name(&self) -> Vec<&str> {
        let mut names = vec![self.name.as_str()];
        if let Some(sort) = &self.sort_name {
            names.push(sort.as_str());
        }
        names.extend(self.aliases.iter().map(String::as_str));
        names
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimaryType {
    Album,
    Single,
    Ep,
    Broadcast,
    Other,
}

impl PrimaryType {
    pub fn parse(text: &str) -> Option<PrimaryType> {
        match text.to_lowercase().as_str() {
            "album" => Some(PrimaryType::Album),
            "single" => Some(PrimaryType::Single),
            "ep" => Some(PrimaryType::Ep),
            "broadcast" => Some(PrimaryType::Broadcast),
            "other" => Some(PrimaryType::Other),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PrimaryType::Album => "Album",
            PrimaryType::Single => "Single",
            PrimaryType::Ep => "EP",
            PrimaryType::Broadcast => "Broadcast",
            PrimaryType::Other => "Other",
        }
    }
}

/// The album as a work, with every issue of it underneath.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReleaseGroup {
    pub mbid: Mbid,
    pub title: String,
    pub artist: ArtistCredit,
    pub first_release_year: Option<i32>,
    pub primary_type: Option<PrimaryType>,
    /// "Soundtrack", "Live", "Compilation" — the words that tell two identically
    /// titled things apart more often than anything else does.
    pub secondary_types: Vec<String>,
    /// MusicBrainz's own free-text tiebreaker, shown verbatim.
    pub disambiguation: Option<String>,
    pub release_count: usize,
}

impl ReleaseGroup {
    /// The one-line subtitle that tells this apart from its neighbours in a list.
    ///
    /// "2000 · Album · Soundtrack · original release" — assembled from whatever
    /// is known, skipping what is not.
    pub fn subtitle(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(year) = self.first_release_year {
            parts.push(year.to_string());
        }
        if let Some(primary) = self.primary_type {
            parts.push(primary.label().to_string());
        }
        parts.extend(self.secondary_types.iter().cloned());
        if let Some(comment) = &self.disambiguation {
            if !comment.is_empty() {
                parts.push(comment.clone());
            }
        }
        parts.join(" · ")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Label {
    pub name: String,
    pub catalog_number: Option<String>,
}

/// One issue of an album.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Release {
    pub mbid: Mbid,
    pub group: Mbid,
    pub title: String,
    pub artist: ArtistCredit,
    /// As MusicBrainz gives it: "2000", "2000-10", "2000-10-02".
    pub date: Option<String>,
    pub country: Option<String>,
    pub labels: Vec<Label>,
    /// "CD", "12\" Vinyl", "Digital Media".
    pub formats: Vec<String>,
    pub track_count: usize,
    pub disambiguation: Option<String>,
    pub packaging: Option<String>,
    pub status: Option<String>,
    pub barcode: Option<String>,
}

impl Release {
    pub fn year(&self) -> Option<i32> {
        self.date
            .as_ref()
            .and_then(|date| date.get(..4))
            .and_then(|year| year.parse().ok())
    }

    /// Whether this issue only ever existed as an object you can hold.
    ///
    /// Drives the ripping friction, and is one half of deciding that an album
    /// was never released digitally at all.
    pub fn is_physical_only(&self) -> bool {
        !self.formats.is_empty()
            && !self
                .formats
                .iter()
                .any(|format| format.eq_ignore_ascii_case("Digital Media"))
    }

    /// A short human name for which edition this is.
    ///
    /// Reads MusicBrainz's disambiguation comment first, because that is where
    /// editors put exactly this, and falls back to whatever the title carries in
    /// brackets. Returns `None` for a plain original issue rather than inventing
    /// the word "original", so that [`super::verdict`] can tell a genuinely
    /// unlabelled release from one an editor called "original".
    pub fn edition(&self) -> Option<String> {
        if let Some(comment) = &self.disambiguation {
            if !comment.trim().is_empty() {
                return Some(comment.trim().to_string());
            }
        }
        let open = self.title.find('(')?;
        let close = self.title[open..].find(')')? + open;
        let inside = self.title[open + 1..close].trim();
        (!inside.is_empty()).then(|| inside.to_string())
    }

    /// "2000 · UK · CD · 10 tracks · Parlophone".
    pub fn subtitle(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(date) = &self.date {
            parts.push(date.clone());
        }
        if let Some(country) = &self.country {
            parts.push(country.clone());
        }
        parts.extend(self.formats.iter().cloned());
        if self.track_count > 0 {
            parts.push(format!("{} tracks", self.track_count));
        }
        if let Some(label) = self.labels.first() {
            if !label.name.is_empty() {
                parts.push(label.name.clone());
            }
        }
        parts.join(" · ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_artist_matches_against_every_name_they_go_by() {
        let artist = ArtistCredit {
            name: "Ryuichi Sakamoto".into(),
            mbid: None,
            sort_name: Some("Sakamoto, Ryuichi".into()),
            aliases: vec!["坂本龍一".into(), "Sakamoto Ryuichi".into()],
        };
        let names = artist.every_name();
        assert!(names.contains(&"Ryuichi Sakamoto"));
        assert!(names.contains(&"Sakamoto, Ryuichi"));
        assert!(names.contains(&"坂本龍一"));
    }

    #[test]
    fn a_release_year_comes_off_any_shape_of_musicbrainz_date() {
        for (date, expected) in [
            ("2000", Some(2000)),
            ("2000-10", Some(2000)),
            ("2000-10-02", Some(2000)),
        ] {
            let release = Release {
                date: Some(date.into()),
                ..Release::default()
            };
            assert_eq!(release.year(), expected, "{date}");
        }
        assert_eq!(Release::default().year(), None);
    }

    #[test]
    fn edition_prefers_the_editors_comment_over_the_title() {
        let release = Release {
            title: "Kid A (Collector's Edition)".into(),
            disambiguation: Some("2009 remaster".into()),
            ..Release::default()
        };
        assert_eq!(release.edition().as_deref(), Some("2009 remaster"));
    }

    #[test]
    fn edition_falls_back_to_the_bracketed_part_of_the_title() {
        let release = Release {
            title: "Kid A (Collector's Edition)".into(),
            ..Release::default()
        };
        assert_eq!(release.edition().as_deref(), Some("Collector's Edition"));
    }

    #[test]
    fn a_plain_release_has_no_edition_rather_than_a_made_up_one() {
        let release = Release {
            title: "Kid A".into(),
            ..Release::default()
        };
        assert_eq!(release.edition(), None);
    }

    #[test]
    fn digital_media_in_the_format_list_means_it_was_not_physical_only() {
        let physical = Release {
            formats: vec!["CD".into(), "12\" Vinyl".into()],
            ..Release::default()
        };
        let digital = Release {
            formats: vec!["Digital Media".into()],
            ..Release::default()
        };
        assert!(physical.is_physical_only());
        assert!(!digital.is_physical_only());
        // Nothing known is not the same as physical only.
        assert!(!Release::default().is_physical_only());
    }
}
