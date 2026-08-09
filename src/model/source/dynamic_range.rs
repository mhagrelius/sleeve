//! The Dynamic Range Database: the master-quality signal.
//!
//! Roughly 193,000 crowd-submitted measurements, one row per pressing, and the
//! only source here that answers in HTML. It is also the most hostile: it
//! returned HTTP 429 on the very first request during development, with a polite
//! User-Agent and with a browser one. [`super::policy`] therefore treats it as
//! the slowest-polled source in the table, it is never on the critical path of a
//! lookup, and every failure it has is neutral.
//!
//! **Attribution is the hard part, not fetching.** The database is keyed by
//! loose artist/album text and holds several rows per album — the 2000 CD, the
//! 2009 remaster, a vinyl rip — with different DR values. Attaching the wrong
//! row to a release moves its score by up to 13 points in the wrong direction,
//! which is worse than having no DR at all. So [`best_match`] refuses to guess:
//! if it cannot tie a row to the specific release, it returns nothing and the
//! offer scores neutrally.

use scraper::{Html, Selector};

use super::{Outcome, Reason, Request, SourceId};

/// Query the database for one album.
pub fn search(artist: &str, album: &str) -> Request {
    Request::get(
        SourceId::DynamicRange,
        format!(
            "https://dr.loudness-war.info/album/list?artist={}&album={}",
            encode(artist),
            encode(album)
        ),
    )
    .header("Accept", "text/html")
}

/// One measured pressing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub artist: String,
    pub album: String,
    pub year: Option<i32>,
    pub dr: u8,
    pub codec: Option<String>,
    /// "CD", "Vinyl", "WEB", "Blu-ray" — what was measured.
    pub source: Option<String>,
}

impl Entry {
    /// How this row is described in the caveat line, so a person can see what
    /// was measured and judge the match themselves.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(year) = self.year {
            parts.push(year.to_string());
        }
        if let Some(source) = &self.source {
            parts.push(source.clone());
        }
        if let Some(codec) = &self.codec {
            parts.push(codec.clone());
        }
        if parts.is_empty() {
            parts.push("an unlabelled pressing".to_string());
        }
        parts.join(", ")
    }
}

pub fn parse(body: &[u8]) -> Outcome<Vec<Entry>> {
    let text = String::from_utf8_lossy(body);
    let document = Html::parse_document(&text);

    let Ok(rows) = Selector::parse("table tr") else {
        return Outcome::Stale(Reason::Malformed("bad row selector".into()));
    };
    let Ok(cells) = Selector::parse("td") else {
        return Outcome::Stale(Reason::Malformed("bad cell selector".into()));
    };

    // No table at all means a block page, an error page, or a redesign — never
    // "this album has no measurements", which comes back as an empty table.
    if document.select(&rows).next().is_none() {
        return Outcome::Stale(Reason::Malformed(
            "no results table in the Dynamic Range DB response".into(),
        ));
    }

    let mut entries = Vec::new();
    for row in document.select(&rows) {
        let columns: Vec<String> = row
            .select(&cells)
            .map(|cell| cell.text().collect::<String>().trim().to_string())
            .collect();
        // Artist, Album, Year, DR, min DR, max DR, Codec, Source.
        if columns.len() < 4 {
            continue;
        }
        let Ok(dr) = columns[3].parse::<u8>() else {
            continue;
        };
        entries.push(Entry {
            artist: columns[0].clone(),
            album: columns[1].clone(),
            year: columns[2].parse().ok().filter(|year| *year > 1000),
            dr,
            codec: columns.get(6).cloned().filter(|c| !c.is_empty()),
            source: columns.get(7).cloned().filter(|s| !s.is_empty()),
        });
    }

    Outcome::of_collection(entries)
}

/// Pick the row that belongs to a specific release, or none at all.
///
/// The refusal to guess is the point. Three rules, in order:
///
/// 1. A row whose year matches the release's wins outright — that is a tie to a
///    specific pressing rather than to an album.
/// 2. Failing that, if every row agrees on a DR value, the album has one master
///    as far as the database knows and the value is safe to use.
/// 3. Otherwise there are several masters and no way to tell which is being
///    sold, so nothing is returned and the offer scores neutrally.
pub fn best_match(entries: &[Entry], release_year: Option<i32>) -> Option<&Entry> {
    if entries.is_empty() {
        return None;
    }

    if let Some(year) = release_year {
        let mut matching = entries.iter().filter(|entry| entry.year == Some(year));
        if let Some(first) = matching.next() {
            // Several rows for the same year — different rips of one pressing —
            // are only usable if they agree.
            if matching.all(|entry| entry.dr == first.dr) {
                return Some(first);
            }
            return None;
        }
    }

    let first = &entries[0];
    entries
        .iter()
        .all(|entry| entry.dr == first.dr)
        .then_some(first)
}

fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &[u8] = br#"<html><body><table>
      <tr><th>Artist</th><th>Album</th><th>Year</th><th>DR</th><th>min</th><th>max</th><th>Codec</th><th>Source</th></tr>
      <tr><td>Radiohead</td><td>Kid A</td><td>2000</td><td>12</td><td>10</td><td>14</td><td>lossless</td><td>CD</td></tr>
      <tr><td>Radiohead</td><td>Kid A</td><td>2009</td><td>6</td><td>5</td><td>8</td><td>lossless</td><td>CD</td></tr>
    </table></body></html>"#;

    #[test]
    fn the_results_table_parses_into_one_entry_per_pressing() {
        let Outcome::Found(entries) = parse(TABLE) else {
            panic!("expected entries");
        };
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].dr, 12);
        assert_eq!(entries[0].year, Some(2000));
        assert_eq!(entries[0].source.as_deref(), Some("CD"));
        assert_eq!(entries[1].dr, 6);
        // The header row has no <td> cells and is skipped without special-casing.
    }

    #[test]
    fn a_year_match_ties_a_measurement_to_a_specific_pressing() {
        let Outcome::Found(entries) = parse(TABLE) else {
            panic!("expected entries");
        };
        assert_eq!(best_match(&entries, Some(2000)).unwrap().dr, 12);
        assert_eq!(best_match(&entries, Some(2009)).unwrap().dr, 6);
    }

    #[test]
    fn disagreeing_masters_with_no_year_to_go_on_yield_nothing() {
        // The rule that matters. Guessing between DR12 and DR6 is a 13-point
        // swing on the ranking, applied to whichever master happened to be
        // listed first. Neutral is the honest answer.
        let Outcome::Found(entries) = parse(TABLE) else {
            panic!("expected entries");
        };
        assert_eq!(best_match(&entries, None), None);
        // And a year that matches nothing is no better than no year.
        assert_eq!(best_match(&entries, Some(2016)), None);
    }

    #[test]
    fn an_album_with_one_master_needs_no_year_to_match() {
        let body = br#"<table>
          <tr><td>A</td><td>B</td><td>1998</td><td>11</td><td>9</td><td>13</td><td>lossless</td><td>CD</td></tr>
          <tr><td>A</td><td>B</td><td>2004</td><td>11</td><td>9</td><td>13</td><td>lossless</td><td>Vinyl</td></tr>
        </table>"#;
        let Outcome::Found(entries) = parse(body) else {
            panic!("expected entries");
        };
        assert_eq!(best_match(&entries, None).unwrap().dr, 11);
    }

    #[test]
    fn an_entry_describes_the_pressing_it_measured() {
        let entry = Entry {
            artist: "Radiohead".into(),
            album: "Kid A".into(),
            year: Some(2000),
            dr: 12,
            codec: Some("lossless".into()),
            source: Some("CD".into()),
        };
        assert_eq!(entry.describe(), "2000, CD, lossless");
    }

    #[test]
    fn no_table_at_all_is_stale_rather_than_no_measurements() {
        // A 429 page, a Cloudflare interstitial and a redesign all look like
        // this, and none of them means the album is unmeasured.
        assert!(matches!(
            parse(b"<html><body><h1>Too Many Requests</h1></body></html>"),
            Outcome::Stale(_)
        ));
    }

    #[test]
    fn an_empty_results_table_is_empty_rather_than_stale() {
        // This one really does mean "no measurements for this album", which is
        // the common case and must score neutrally without reporting a fault.
        let body = br#"<table><tr><th>Artist</th><th>DR</th></tr></table>"#;
        assert_eq!(parse(body), Outcome::Empty);
    }

    #[test]
    fn a_row_with_an_unreadable_dr_is_skipped_not_defaulted() {
        // A defaulted zero would read as the most compressed master ever
        // measured and cost the offer five points.
        let body = br#"<table>
          <tr><td>A</td><td>B</td><td>2000</td><td>n/a</td></tr>
          <tr><td>A</td><td>B</td><td>2001</td><td>9</td></tr>
        </table>"#;
        let Outcome::Found(entries) = parse(body) else {
            panic!("expected entries");
        };
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].dr, 9);
    }

    #[test]
    fn the_query_encodes_spaces_the_way_the_form_does() {
        let request = search("Miles Davis", "Kind of Blue");
        assert!(request.url.contains("artist=Miles+Davis"));
        assert!(request.url.contains("album=Kind+of+Blue"));
    }
}
