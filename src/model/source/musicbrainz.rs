//! MusicBrainz: canonical identity.
//!
//! The only required source. Everything else is keyed off what this returns —
//! an MBID is what makes the Cover Art Archive line up exactly, and the
//! release-group/release split it models is the one this whole application is
//! built around.
//!
//! Two rules their operators enforce rather than request: no more than one
//! request a second, and a `User-Agent` naming the application and a way to
//! contact whoever runs it. Breaching either gets the User-Agent blocked, so the
//! interval lives in [`super::policy`] and the header is not optional here.

use serde_json::Value;

use super::{parse_json, Outcome, Reason, Request, SourceId};
use crate::model::album::{ArtistCredit, Label, Mbid, PrimaryType, Release, ReleaseGroup};
use crate::model::offer::{Acquisition, Delivery, Offer, Vendor};

const BASE: &str = "https://musicbrainz.org/ws/2";

/// Search for release groups matching an artist and album.
///
/// The query is fielded — `artist:` and `releasegroup:` — because an unfielded
/// search for "kid a radiohead" scores every release group containing either
/// word and buries the answer. Fielding it makes MusicBrainz's own relevance
/// score worth using as the primary signal, which is what
/// [`crate::model::search`] then re-ranks.
pub fn search_release_groups(artist: &str, album: &str, user_agent: &str) -> Request {
    let mut clauses = Vec::new();
    if !album.trim().is_empty() {
        clauses.push(format!("releasegroup:({})", escape_lucene(album)));
    }
    if !artist.trim().is_empty() {
        clauses.push(format!("artist:({})", escape_lucene(artist)));
    }
    let query = if clauses.is_empty() {
        "*".to_string()
    } else {
        clauses.join(" AND ")
    };

    Request::get(
        SourceId::MusicBrainz,
        format!(
            "{BASE}/release-group?query={}&fmt=json&limit=25",
            encode(&query)
        ),
    )
    .header("User-Agent", user_agent)
    .header("Accept", "application/json")
}

/// Every release group by one artist.
///
/// Feeds the "similarly named releases by the same artist" half of the
/// near-miss list, which is how a person finds the edition they actually meant.
pub fn browse_artist_release_groups(artist: &Mbid, user_agent: &str) -> Request {
    Request::get(
        SourceId::MusicBrainz,
        format!(
            "{BASE}/release-group?artist={}&fmt=json&limit=100",
            artist.as_str()
        ),
    )
    .header("User-Agent", user_agent)
    .header("Accept", "application/json")
}

/// Every release in one release group.
///
/// The browse endpoint rather than a release-group lookup with `inc=releases`,
/// because only browse reliably carries `media` — and without `media` there is
/// no format and no track count, which are the two things that tell a CD issue
/// from a vinyl reissue in the drill-down list.
pub fn browse_releases(group: &Mbid, user_agent: &str) -> Request {
    Request::get(
        SourceId::MusicBrainz,
        format!(
            "{BASE}/release?release-group={}&inc=media+labels+artist-credits&fmt=json&limit=100",
            group.as_str()
        ),
    )
    .header("User-Agent", user_agent)
    .header("Accept", "application/json")
}

/// Every link editors have attached to one release.
///
/// This is the most valuable request in the application and it took a while to
/// notice. MusicBrainz editors record `purchase for download` relations pointing
/// at Bandcamp, Qobuz, Bleep, Boomkat, Beatport, Juno and the rest — which means
/// the shops that refuse an HTTP client outright, and the one that wants a paid
/// subscription before it will answer, are all reachable through a source that
/// needs no credentials at all.
///
/// It is an index, not a check. A link records that somebody once saw the album
/// for sale there, and coverage depends on an editor having bothered: of four
/// albums sampled while building this, one had five links, one had three, one had
/// one, and Burial's *Untrue* had none. Absence means "nobody added one", never
/// "not sold there". That is why these offers are marked
/// [`Provenance::Indexed`].
pub fn release_urls(release: &Mbid, user_agent: &str) -> Request {
    Request::get(
        SourceId::MusicBrainz,
        format!("{BASE}/release/{}?inc=url-rels&fmt=json", release.as_str()),
    )
    .header("User-Agent", user_agent)
    .header("Accept", "application/json")
}

/// Turn a release's link relations into offers.
///
/// No prices: a relation is a URL and nothing else. That costs less than it
/// sounds, because price is reported and never scored — a tier-A Bleep offer with
/// an unknown price still ranks exactly where it belongs.
pub fn parse_purchase_links(body: &[u8]) -> Outcome<Vec<Offer>> {
    let root = match parse_json(SourceId::MusicBrainz, body) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };
    let Some(relations) = root.get("relations").and_then(Value::as_array) else {
        return Outcome::Stale(Reason::Malformed("no relations array".into()));
    };

    let mut offers: Vec<Offer> = Vec::new();
    for relation in relations {
        let Some(kind) = relation.get("type").and_then(Value::as_str) else {
            continue;
        };
        let Some(url) = relation
            .get("url")
            .and_then(|url| url.get("resource"))
            .and_then(Value::as_str)
        else {
            continue;
        };

        let offer = match kind {
            "purchase for download" => vendor_for_download(url).map(|vendor| {
                Offer::new(vendor, Acquisition::Purchase, delivery_for_store(vendor))
            }),
            // "free streaming" is the ad-supported tier of a service whose paid
            // tier is what gets ranked; both point at the same catalogue entry.
            "streaming" | "free streaming" => vendor_for_streaming(url).map(|vendor| {
                Offer::new(
                    vendor,
                    Acquisition::Subscription,
                    streaming_delivery(vendor),
                )
            }),
            _ => None,
        };

        let Some(offer) = offer else { continue };
        // One shop, one offer, even where an editor has added a link per
        // storefront — the .com and the .co.uk of a shop are one shop.
        if offers
            .iter()
            .any(|existing| existing.vendor == offer.vendor)
        {
            continue;
        }
        offers.push(offer.with_url(url).indexed());
    }

    Outcome::of_collection(offers)
}

/// What an editor's Discogs link points at.
///
/// Both shapes occur and they are not interchangeable: a release-group relation
/// gives `/master/21501`, a release relation gives `/release/35429287`, and
/// asking the wrong endpoint for either is a 404. Discogs release 21501 does not
/// exist even though master 21501 is *Kid A*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscogsLink {
    /// The album as a work. Needs a version list before it can be priced.
    Master(u64),
    /// One pressing, which can be priced directly.
    Release(u64),
}

/// The Discogs record an editor has already matched to this album.
///
/// Better than searching Discogs by text: an editor tied these two together by
/// hand, so it cannot land on a tribute album the way a fuzzy title match can.
/// A release link is preferred over a master when both are present — it is the
/// more specific of the two and saves a request.
pub fn parse_discogs_link(body: &[u8]) -> Option<DiscogsLink> {
    let root: Value = serde_json::from_slice(body).ok()?;
    let urls: Vec<&str> = root
        .get("relations")?
        .as_array()?
        .iter()
        .filter(|relation| relation.get("type").and_then(Value::as_str) == Some("discogs"))
        .filter_map(|relation| relation.get("url")?.get("resource")?.as_str())
        .collect();

    let id_after = |url: &str, segment: &str| -> Option<u64> {
        let (_, tail) = url.rsplit_once(segment)?;
        tail.split(['/', '?', '#']).next()?.parse().ok()
    };

    urls.iter()
        .find_map(|url| id_after(url, "/release/").map(DiscogsLink::Release))
        .or_else(|| {
            urls.iter()
                .find_map(|url| id_after(url, "/master/").map(DiscogsLink::Master))
        })
}

/// Release-group links, for the Discogs master when the release has none.
pub fn release_group_urls(group: &Mbid, user_agent: &str) -> Request {
    Request::get(
        SourceId::MusicBrainz,
        format!(
            "{BASE}/release-group/{}?inc=url-rels&fmt=json",
            group.as_str()
        ),
    )
    .header("User-Agent", user_agent)
    .header("Accept", "application/json")
}

/// Which shop a purchase link points at.
fn vendor_for_download(url: &str) -> Option<Vendor> {
    let host = host_of(url)?;
    Some(match host.as_str() {
        h if h.ends_with("bandcamp.com") => Vendor::Bandcamp,
        h if h.ends_with("qobuz.com") => Vendor::QobuzStore,
        h if h.ends_with("bleep.com") => Vendor::Bleep,
        h if h.ends_with("boomkat.com") => Vendor::Boomkat,
        h if h.ends_with("prestomusic.com") || h.ends_with("prestoclassical.co.uk") => {
            Vendor::PrestoMusic
        }
        h if h.ends_with("hdtracks.com") => Vendor::HdTracks,
        h if h.ends_with("beatport.com") => Vendor::Beatport,
        h if h.ends_with("junodownload.com") => Vendor::JunoDownload,
        h if h.ends_with("7digital.com") => Vendor::SevenDigital,
        h if h.ends_with("itunes.apple.com") || h.ends_with("music.apple.com") => Vendor::ITunes,
        // Google Play Music shut down in 2020 and Amazon's store links carry no
        // price we can read. Neither is an offer worth ranking.
        _ => return None,
    })
}

fn vendor_for_streaming(url: &str) -> Option<Vendor> {
    let host = host_of(url)?;
    Some(match host.as_str() {
        h if h.ends_with("spotify.com") => Vendor::Spotify,
        h if h.ends_with("deezer.com") => Vendor::Deezer,
        h if h.ends_with("music.apple.com") => Vendor::AppleMusic,
        h if h.ends_with("tidal.com") => Vendor::Tidal,
        h if h.ends_with("music.amazon.com") || h.ends_with("music.amazon.co.uk") => {
            Vendor::AmazonMusicHd
        }
        h if h.ends_with("music.youtube.com") => Vendor::YouTubeMusic,
        h if h.ends_with("qobuz.com") => Vendor::Qobuz,
        _ => return None,
    })
}

/// What a download store sells, when all we have is that it sells it.
///
/// Every shop in the list above sells lossless; iTunes is the one exception and
/// is the reason this is a match rather than a constant.
fn delivery_for_store(vendor: Vendor) -> Delivery {
    match vendor {
        Vendor::ITunes => Delivery::lossy("AAC 256"),
        // No bit depth or sample rate claimed, because a link does not say. The
        // format bonus is a tiebreaker worth two points; inventing 24-bit to
        // collect it would be making the number up.
        _ => Delivery {
            lossless: true,
            codec: Some("FLAC".into()),
            ..Delivery::default()
        },
    }
}

fn streaming_delivery(vendor: Vendor) -> Delivery {
    match vendor {
        Vendor::YouTubeMusic => Delivery::lossy("Opus 256"),
        Vendor::Spotify => Delivery::lossy("Ogg Vorbis 320"),
        _ => Delivery {
            lossless: true,
            codec: Some("FLAC".into()),
            ..Delivery::default()
        },
    }
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?;
    Some(host.trim_start_matches("www.").to_lowercase())
}

/// One artist with every alias they have.
pub fn lookup_artist_aliases(artist: &Mbid, user_agent: &str) -> Request {
    Request::get(
        SourceId::MusicBrainz,
        format!("{BASE}/artist/{}?inc=aliases&fmt=json", artist.as_str()),
    )
    .header("User-Agent", user_agent)
    .header("Accept", "application/json")
}

/// A release group as MusicBrainz scored it.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredGroup {
    pub group: ReleaseGroup,
    /// MusicBrainz's own relevance, 0–100. Absent on a browse, which is not a
    /// search — those default to zero and are re-ranked locally on text alone.
    pub score: u8,
}

pub fn parse_release_groups(body: &[u8]) -> Outcome<Vec<ScoredGroup>> {
    let root = match parse_json(SourceId::MusicBrainz, body) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };

    let Some(items) = root.get("release-groups").and_then(Value::as_array) else {
        return Outcome::Stale(Reason::Malformed(
            "no release-groups array in the response".into(),
        ));
    };

    let groups: Vec<ScoredGroup> = items
        .iter()
        .filter_map(|item| {
            let mbid = item.get("id").and_then(Value::as_str)?;
            Some(ScoredGroup {
                group: ReleaseGroup {
                    mbid: Mbid::new(mbid),
                    title: string(item, "title").unwrap_or_default(),
                    artist: artist_credit(item),
                    first_release_year: string(item, "first-release-date")
                        .as_deref()
                        .and_then(year_of),
                    primary_type: string(item, "primary-type")
                        .as_deref()
                        .and_then(PrimaryType::parse),
                    secondary_types: item
                        .get("secondary-types")
                        .and_then(Value::as_array)
                        .map(|types| {
                            types
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default(),
                    disambiguation: string(item, "disambiguation").filter(|s| !s.is_empty()),
                    release_count: item.get("count").and_then(Value::as_u64).unwrap_or(0) as usize,
                },
                score: item
                    .get("score")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .min(100) as u8,
            })
        })
        .collect();

    Outcome::of_collection(groups)
}

pub fn parse_releases(group: &Mbid, body: &[u8]) -> Outcome<Vec<Release>> {
    let root = match parse_json(SourceId::MusicBrainz, body) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };

    let Some(items) = root.get("releases").and_then(Value::as_array) else {
        return Outcome::Stale(Reason::Malformed(
            "no releases array in the response".into(),
        ));
    };

    let releases: Vec<Release> = items
        .iter()
        .filter_map(|item| {
            let mbid = item.get("id").and_then(Value::as_str)?;
            let media = item.get("media").and_then(Value::as_array);
            Some(Release {
                mbid: Mbid::new(mbid),
                group: group.clone(),
                title: string(item, "title").unwrap_or_default(),
                artist: artist_credit(item),
                date: string(item, "date").filter(|date| !date.is_empty()),
                country: string(item, "country"),
                labels: item
                    .get("label-info")
                    .and_then(Value::as_array)
                    .map(|infos| {
                        infos
                            .iter()
                            .filter_map(|info| {
                                let name = info
                                    .get("label")
                                    .and_then(|label| string(label, "name"))
                                    .unwrap_or_default();
                                let catalog = string(info, "catalog-number");
                                (!name.is_empty() || catalog.is_some()).then_some(Label {
                                    name,
                                    catalog_number: catalog,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                formats: media
                    .map(|media| {
                        media
                            .iter()
                            .filter_map(|medium| string(medium, "format"))
                            .collect()
                    })
                    .unwrap_or_default(),
                track_count: media
                    .map(|media| {
                        media
                            .iter()
                            .filter_map(|medium| medium.get("track-count").and_then(Value::as_u64))
                            .sum::<u64>() as usize
                    })
                    .unwrap_or(0),
                disambiguation: string(item, "disambiguation").filter(|s| !s.is_empty()),
                packaging: string(item, "packaging"),
                status: string(item, "status"),
                barcode: string(item, "barcode").filter(|s| !s.is_empty()),
            })
        })
        .collect();

    Outcome::of_collection(releases)
}

/// Every name one artist goes by, for the alias-aware match.
pub fn parse_artist_aliases(body: &[u8]) -> Outcome<Vec<String>> {
    let root = match parse_json(SourceId::MusicBrainz, body) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };
    if root.get("id").is_none() {
        return Outcome::Stale(Reason::Malformed("no artist in the response".into()));
    }

    let mut names = Vec::new();
    if let Some(name) = string(&root, "name") {
        names.push(name);
    }
    if let Some(sort) = string(&root, "sort-name") {
        names.push(sort);
    }
    if let Some(aliases) = root.get("aliases").and_then(Value::as_array) {
        // Both the alias and its sort-name: a Japanese artist's alias list holds
        // the native script under `name` and the romanisation under `sort-name`,
        // and someone might type either.
        for alias in aliases {
            names.extend(string(alias, "name"));
            names.extend(string(alias, "sort-name"));
        }
    }
    names.sort();
    names.dedup();
    Outcome::of_collection(names)
}

// -- helpers ----------------------------------------------------------------

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn year_of(date: &str) -> Option<i32> {
    date.get(..4).and_then(|year| year.parse().ok())
}

fn artist_credit(item: &Value) -> ArtistCredit {
    let Some(credits) = item.get("artist-credit").and_then(Value::as_array) else {
        return ArtistCredit::default();
    };

    // A credit is a list of fragments with joinphrases — "Simon" + " & " +
    // "Garfunkel". The display name is the whole phrase; the MBID is the first
    // artist's, which is the one a browse can be hung off.
    let mut name = String::new();
    let mut mbid = None;
    let mut sort_name = None;
    for credit in credits {
        let artist = credit.get("artist");
        let fragment = string(credit, "name")
            .or_else(|| artist.and_then(|a| string(a, "name")))
            .unwrap_or_default();
        name.push_str(&fragment);
        if let Some(join) = string(credit, "joinphrase") {
            name.push_str(&join);
        }
        if mbid.is_none() {
            mbid = artist.and_then(|a| string(a, "id")).map(Mbid::new);
            sort_name = artist.and_then(|a| string(a, "sort-name"));
        }
    }

    ArtistCredit {
        name,
        mbid,
        sort_name,
        aliases: Vec::new(),
    }
}

/// Escape the characters Lucene treats as syntax.
///
/// An unescaped `:` in a title — every "Volume 1: Something" — turns the rest of
/// the query into a field name and the search returns nothing at all.
fn escape_lucene(text: &str) -> String {
    const SPECIAL: &[char] = &[
        '+', '-', '&', '|', '!', '(', ')', '{', '}', '[', ']', '^', '"', '~', '*', '?', ':', '\\',
        '/',
    ];
    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        if SPECIAL.contains(&ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Percent-encode for a query string.
///
/// Twenty lines instead of a dependency, for one job in one place. The unreserved
/// set is RFC 3986's; everything else, including the space, goes to `%XX`.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const UA: &str = "Sleeve/0.1 ( matthew@hagreli.us )";

    #[test]
    fn a_search_is_fielded_on_both_artist_and_album() {
        let request = search_release_groups("Radiohead", "Kid A", UA);
        assert!(request.url.contains("releasegroup%3A%28Kid%20A%29"));
        assert!(request.url.contains("artist%3A%28Radiohead%29"));
        assert!(request.url.contains("fmt=json"));
    }

    #[test]
    fn a_colon_in_a_title_is_escaped_rather_than_read_as_a_field() {
        // "Vol. 2: Release" unescaped makes Lucene look for a field called
        // "2" and return nothing. Classical and soundtrack titles are full of
        // these, which is exactly where disambiguation matters most.
        let request = search_release_groups("", "Kill Bill Vol. 1: Original Soundtrack", UA);
        assert!(
            request.url.contains("%5C%3A"),
            "colon not escaped: {}",
            request.url
        );
    }

    #[test]
    fn searching_with_only_an_album_still_builds_a_valid_query() {
        let request = search_release_groups("", "Kid A", UA);
        assert!(request.url.contains("releasegroup"));
        assert!(!request.url.contains("artist%3A"));
    }

    #[test]
    fn every_request_carries_a_contactable_user_agent() {
        // Not politeness — MusicBrainz blocks a User-Agent without one.
        let requests = [
            search_release_groups("a", "b", UA),
            browse_releases(&Mbid::new("x"), UA),
            browse_artist_release_groups(&Mbid::new("x"), UA),
            lookup_artist_aliases(&Mbid::new("x"), UA),
        ];
        for request in requests {
            let agent = request
                .headers
                .iter()
                .find(|(name, _)| name == "User-Agent")
                .map(|(_, value)| value.clone())
                .expect("a User-Agent");
            assert!(agent.contains('@'), "no contact address in {agent}");
        }
    }

    #[test]
    fn a_search_response_parses_into_scored_groups() {
        let body = br#"{"release-groups":[{
            "id":"b1392450-e666-3926-a536-22c65f834433",
            "score":100,
            "title":"Kid A",
            "first-release-date":"2000-10-02",
            "primary-type":"Album",
            "secondary-types":["Live"],
            "disambiguation":"",
            "count":24,
            "artist-credit":[{"name":"Radiohead","artist":{
                "id":"a74b1b7f-71a5-4011-9441-d0b5e4122711",
                "name":"Radiohead","sort-name":"Radiohead"}}]
        }]}"#;
        let Outcome::Found(groups) = parse_release_groups(body) else {
            panic!("expected groups");
        };
        assert_eq!(groups.len(), 1);
        let found = &groups[0];
        assert_eq!(found.score, 100);
        assert_eq!(found.group.title, "Kid A");
        assert_eq!(found.group.first_release_year, Some(2000));
        assert_eq!(found.group.primary_type, Some(PrimaryType::Album));
        assert_eq!(found.group.secondary_types, vec!["Live".to_string()]);
        assert_eq!(found.group.release_count, 24);
        assert_eq!(found.group.artist.name, "Radiohead");
        assert!(found.group.disambiguation.is_none());
    }

    #[test]
    fn a_multi_artist_credit_joins_into_one_display_name() {
        let body = br#"{"release-groups":[{"id":"x","artist-credit":[
            {"name":"Simon","joinphrase":" & ","artist":{"id":"1","name":"Simon"}},
            {"name":"Garfunkel","artist":{"id":"2","name":"Garfunkel"}}]}]}"#;
        let Outcome::Found(groups) = parse_release_groups(body) else {
            panic!("expected groups");
        };
        assert_eq!(groups[0].group.artist.name, "Simon & Garfunkel");
        // The MBID is the first artist's, so a browse off it works.
        assert_eq!(groups[0].group.artist.mbid.as_ref().unwrap().as_str(), "1");
    }

    #[test]
    fn releases_carry_the_facts_that_tell_two_pressings_apart() {
        let body = br#"{"releases":[{
            "id":"r1","title":"Kid A","date":"2000-10-02","country":"GB",
            "status":"Official","packaging":"Jewel Case","barcode":"724352727520",
            "disambiguation":"",
            "label-info":[{"catalog-number":"7243 5 27753 2 3","label":{"name":"Parlophone"}}],
            "media":[{"format":"CD","track-count":10},{"format":"CD","track-count":2}]
        }]}"#;
        let Outcome::Found(releases) = parse_releases(&Mbid::new("g1"), body) else {
            panic!("expected releases");
        };
        let release = &releases[0];
        assert_eq!(release.group.as_str(), "g1");
        assert_eq!(release.country.as_deref(), Some("GB"));
        assert_eq!(release.formats, vec!["CD".to_string(), "CD".to_string()]);
        // Track count sums across discs — a two-disc set is not a ten-track album.
        assert_eq!(release.track_count, 12);
        assert_eq!(release.labels[0].name, "Parlophone");
        assert!(release.is_physical_only());
    }

    #[test]
    fn aliases_include_native_script_and_romanisation() {
        // A raw *byte* string cannot hold non-ASCII, and the whole point of this
        // case is the non-ASCII, so the fixture is text converted at the seam.
        let body = r#"{"id":"a1","name":"Ryuichi Sakamoto","sort-name":"Sakamoto, Ryuichi",
            "aliases":[{"name":"坂本龍一","sort-name":"坂本龍一"},
                       {"name":"Sakamoto Ryuichi","sort-name":"Sakamoto, Ryuichi"}]}"#;
        let Outcome::Found(names) = parse_artist_aliases(body.as_bytes()) else {
            panic!("expected aliases");
        };
        assert!(names.iter().any(|n| n == "坂本龍一"));
        assert!(names.iter().any(|n| n == "Ryuichi Sakamoto"));
        assert!(names.iter().any(|n| n == "Sakamoto Ryuichi"));
        // Deduplicated: "Sakamoto, Ryuichi" appears twice in the input.
        let count = names.iter().filter(|n| *n == "Sakamoto, Ryuichi").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn no_results_is_empty_and_a_changed_shape_is_stale() {
        assert_eq!(
            parse_release_groups(br#"{"release-groups":[]}"#),
            Outcome::Empty
        );
        assert!(matches!(
            parse_release_groups(br#"{"something-else":[]}"#),
            Outcome::Stale(_)
        ));
        assert!(matches!(
            parse_release_groups(b"<html>blocked</html>"),
            Outcome::Stale(_)
        ));
    }

    #[test]
    fn a_release_group_missing_its_id_is_skipped_rather_than_defaulted() {
        // A record with no MBID cannot be drilled into or have art fetched, so
        // it is not a candidate. Better dropped than shown as a dead row.
        let body = br#"{"release-groups":[{"title":"No Id"},{"id":"x","title":"Fine"}]}"#;
        let Outcome::Found(groups) = parse_release_groups(body) else {
            panic!("expected groups");
        };
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].group.title, "Fine");
    }
}
