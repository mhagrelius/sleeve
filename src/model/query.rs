//! Turning what someone typed into something comparable.
//!
//! Two jobs, both of which exist because people do not type catalogue entries.
//! Folding makes "Bjork" match "Björk" and "BEYONCE" match "Beyoncé". Splitting
//! the edition hint off makes "Kid A" match "Kid A (Deluxe Edition)" at full
//! strength — the bracketed part is not a difference in the album's name, it is
//! a fact about the pressing, and treating it as a difference is what makes a
//! search for an album fail to find its own deluxe edition.

/// A search as typed, and as compared.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub artist: String,
    pub album: String,
}

impl Query {
    pub fn new(artist: &str, album: &str) -> Self {
        Query {
            artist: artist.trim().to_string(),
            album: album.trim().to_string(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.artist.is_empty() && self.album.is_empty()
    }

    /// The album title with any edition hint removed.
    pub fn album_base(&self) -> String {
        strip_edition(&self.album).0
    }

    /// The edition the person asked for, if they named one.
    ///
    /// Someone who types "Kid A (Deluxe)" wants the deluxe edition, and that is
    /// worth knowing when several editions match equally well.
    pub fn edition_hint(&self) -> Option<String> {
        strip_edition(&self.album).1
    }
}

/// Split a title into its name and its bracketed edition hint.
///
/// Only trailing brackets are treated as an edition. A bracket in the middle of
/// a title is part of the title — "(What's the Story) Morning Glory?" is not an
/// edition of "Morning Glory?".
pub fn strip_edition(title: &str) -> (String, Option<String>) {
    let trimmed = title.trim();
    let Some(last) = trimmed.chars().last() else {
        return (String::new(), None);
    };
    let close = match last {
        ')' => ')',
        ']' => ']',
        _ => return (trimmed.to_string(), None),
    };
    let open = if close == ')' { '(' } else { '[' };

    let Some(index) = trimmed.rfind(open) else {
        return (trimmed.to_string(), None);
    };
    // A title that is nothing but a bracketed phrase keeps it — stripping it
    // would leave an empty query.
    if index == 0 {
        return (trimmed.to_string(), None);
    }

    let base = trimmed[..index].trim().to_string();
    let hint = trimmed[index + 1..trimmed.len() - close.len_utf8()]
        .trim()
        .to_string();
    if base.is_empty() || hint.is_empty() {
        return (trimmed.to_string(), None);
    }
    (base, Some(hint))
}

/// Casefold, strip accents, and reduce punctuation to spaces.
///
/// The comparable form of a string. Everything that reaches a similarity
/// function has been through this, on both sides.
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_was_space = true;

    let push = |ch: char, out: &mut String, last_was_space: &mut bool| {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
            *last_was_space = false;
        } else if !*last_was_space {
            out.push(' ');
            *last_was_space = true;
        }
    };

    for ch in text.chars() {
        match deaccent(ch) {
            Some(replacement) => {
                for ch in replacement.chars() {
                    push(ch, &mut out, &mut last_was_space);
                }
            }
            None => push(ch, &mut out, &mut last_was_space),
        }
    }
    out.trim_end().to_string()
}

/// Map an accented Latin character to its unaccented form.
///
/// A table rather than Unicode normalisation, because the job is narrow: the
/// scripts this needs to fold are the Latin ones, and everything else —
/// Cyrillic, Han, Hangul, Greek — matches exactly against the alias list
/// MusicBrainz already supplies, so decomposing it would gain nothing. Twenty
/// lines of table against a normalisation dependency, for a search box.
fn deaccent(ch: char) -> Option<&'static str> {
    Some(match ch {
        'À'..='Å' | 'à'..='å' | 'Ā' | 'ā' | 'Ă' | 'ă' | 'Ą' | 'ą' => "a",
        'Æ' | 'æ' => "ae",
        'Ç' | 'ç' | 'Ć' | 'ć' | 'Č' | 'č' => "c",
        'Ď' | 'ď' | 'Đ' | 'đ' | 'Ð' | 'ð' => "d",
        'È'..='Ë' | 'è'..='ë' | 'Ē' | 'ē' | 'Ę' | 'ę' | 'Ě' | 'ě' => "e",
        'Ì'..='Ï' | 'ì'..='ï' | 'Ī' | 'ī' | 'Į' | 'į' => "i",
        'Ł' | 'ł' => "l",
        'Ñ' | 'ñ' | 'Ń' | 'ń' | 'Ň' | 'ň' => "n",
        'Ò'..='Ö' | 'Ø' | 'ò'..='ö' | 'ø' | 'Ō' | 'ō' | 'Ő' | 'ő' => "o",
        'Œ' | 'œ' => "oe",
        'Ř' | 'ř' => "r",
        'Ś' | 'ś' | 'Š' | 'š' | 'Ş' | 'ş' => "s",
        'ß' => "ss",
        'Ť' | 'ť' | 'Ţ' | 'ţ' => "t",
        'Ù'..='Ü' | 'ù'..='ü' | 'Ū' | 'ū' | 'Ů' | 'ů' | 'Ű' | 'ű' => "u",
        'Ý' | 'ý' | 'ÿ' | 'Ÿ' => "y",
        'Ź' | 'ź' | 'Ż' | 'ż' | 'Ž' | 'ž' => "z",
        // Everything else — including every non-Latin script — is used as it
        // stands. Folding Cyrillic or Han would only break a match that already
        // works, because MusicBrainz supplies those names verbatim as aliases.
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding_removes_accents_case_and_punctuation() {
        assert_eq!(fold("Björk"), "bjork");
        assert_eq!(fold("BEYONCÉ"), "beyonce");
        assert_eq!(fold("Sigur Rós"), "sigur ros");
        assert_eq!(fold("Motörhead"), "motorhead");
        assert_eq!(fold("Blue Öyster Cult"), "blue oyster cult");
    }

    #[test]
    fn punctuation_becomes_a_single_separator_rather_than_vanishing() {
        // "R.E.M." must not fold to "rem" — that would match "REM" and also
        // "R E M", which is right — but it must not run words together either.
        assert_eq!(fold("Vol. 2: The Release"), "vol 2 the release");
        assert_eq!(fold("Sgt. Pepper's"), "sgt pepper s");
        assert_eq!(fold("  spaced   out  "), "spaced out");
    }

    #[test]
    fn non_latin_scripts_pass_through_untouched() {
        // They are matched against MusicBrainz's alias list verbatim, so folding
        // them would only break the match.
        assert_eq!(fold("坂本龍一"), "坂本龍一");
        assert_eq!(fold("Мумий Тролль"), "мумий тролль");
    }

    #[test]
    fn a_trailing_bracket_is_an_edition_hint() {
        assert_eq!(
            strip_edition("Kid A (Deluxe Edition)"),
            ("Kid A".into(), Some("Deluxe Edition".into()))
        );
        assert_eq!(
            strip_edition("OK Computer [OKNOTOK 1997 2017]"),
            ("OK Computer".into(), Some("OKNOTOK 1997 2017".into()))
        );
    }

    #[test]
    fn a_bracket_inside_a_title_is_part_of_the_title() {
        // Stripping this would search for "Morning Glory?" and rank the real
        // album as a near miss of itself.
        assert_eq!(
            strip_edition("(What's the Story) Morning Glory?"),
            ("(What's the Story) Morning Glory?".into(), None)
        );
    }

    #[test]
    fn a_title_with_no_brackets_is_left_alone() {
        assert_eq!(strip_edition("Kid A"), ("Kid A".into(), None));
        assert_eq!(strip_edition(""), (String::new(), None));
    }

    #[test]
    fn an_empty_bracket_is_not_an_edition() {
        assert_eq!(strip_edition("Kid A ()"), ("Kid A ()".into(), None));
    }

    #[test]
    fn a_query_exposes_the_edition_someone_asked_for() {
        let query = Query::new("Radiohead", "Kid A (Deluxe)");
        assert_eq!(query.album_base(), "Kid A");
        assert_eq!(query.edition_hint().as_deref(), Some("Deluxe"));
    }
}
