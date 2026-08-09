//! Every source, against a body it really sent.
//!
//! The fixtures in `tests/fixtures/` were recorded from the live APIs. Nothing
//! here opens a socket: the source layer is a pair of pure functions per source,
//! so a recorded body is a complete test of everything but the transport.
//!
//! Several of these assert on facts that are inconvenient — Bandcamp having
//! nothing but cover versions of a famous album, Qobuz refusing an
//! unauthenticated call. Those are the cases worth pinning down, because they
//! are the ones a hand-written fixture would have got wrong.

use sleeve::model::offer::{Acquisition, Provenance, Vendor};
use sleeve::model::source::{
    bandcamp, dynamic_range, itunes, musicbrainz, odesli, qobuz, Outcome, Reason,
};
use sleeve::model::tier::{tier_of, Tier};

fn fixture(path: &str) -> Vec<u8> {
    let full = format!("{}/tests/fixtures/{path}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&full).unwrap_or_else(|error| panic!("fixture {full}: {error}"))
}

fn text(path: &str) -> String {
    String::from_utf8(fixture(path)).expect("utf-8 fixture")
}

// -- MusicBrainz -------------------------------------------------------------

#[test]
fn the_recorded_search_puts_the_right_kid_a_first() {
    let Outcome::Found(groups) =
        musicbrainz::parse_release_groups(&fixture("musicbrainz/search-kid-a.json"))
    else {
        panic!("expected release groups");
    };

    // Twenty results, and the real record is not alone: the same response holds
    // "KID A MNESIA", "The Kid A Theory", "Kid 17" and "Kid Alive", all by
    // artists MusicBrainz scored above 80.
    assert!(groups.len() > 5);
    let best = &groups[0];
    assert_eq!(best.group.title, "Kid A");
    assert_eq!(best.group.artist.name, "Radiohead");
    assert_eq!(best.score, 100);
    assert!(best.group.release_count > 1);
}

#[test]
fn the_recorded_releases_carry_the_pressings_apart() {
    let group = sleeve::model::album::Mbid::new(text("musicbrainz/kid-a-group-mbid.txt").trim());
    let Outcome::Found(releases) =
        musicbrainz::parse_releases(&group, &fixture("musicbrainz/releases-kid-a.json"))
    else {
        panic!("expected releases");
    };

    // Thirty issues of one album, which is the whole reason the drill-down
    // exists rather than a single "Kid A" row.
    assert!(releases.len() > 20, "only {} releases", releases.len());

    // A real spread of formats, and the physical/digital split the ripping
    // friction depends on.
    let formats: Vec<&str> = releases
        .iter()
        .flat_map(|release| release.formats.iter().map(String::as_str))
        .collect();
    assert!(formats.contains(&"CD"));
    assert!(formats.contains(&"Digital Media"));
    assert!(formats.iter().any(|format| format.contains("Vinyl")));
    assert!(releases.iter().any(|release| release.is_physical_only()));
    assert!(releases.iter().any(|release| !release.is_physical_only()));

    // Every release belongs to the group it was browsed from.
    assert!(releases.iter().all(|release| release.group == group));
    // And a two-disc issue really does sum its tracks.
    assert!(releases.iter().any(|release| release.track_count > 10));
}

#[test]
fn the_recorded_purchase_relations_reach_the_shops_no_api_will_talk_to() {
    // The request that makes Sleeve useful with no credentials at all. Bleep
    // refuses a plain HTTP client and Qobuz wants a paid account; both arrive
    // here for free, from a source already being called.
    let Outcome::Found(offers) =
        musicbrainz::parse_purchase_links(&fixture("musicbrainz/release-urls-kid-a.json"))
    else {
        panic!("expected offers");
    };

    let vendors: Vec<Vendor> = offers.iter().map(|offer| offer.vendor).collect();
    assert!(vendors.contains(&Vendor::Bleep), "{vendors:?}");
    assert!(vendors.contains(&Vendor::QobuzStore), "{vendors:?}");
    assert!(vendors.contains(&Vendor::Bandcamp), "{vendors:?}");
    assert!(vendors.contains(&Vendor::ITunes), "{vendors:?}");

    // Google Play Music shut down in 2020; its link is still in the data and
    // must not become an offer.
    assert!(!offers.iter().any(|offer| offer
        .url
        .as_deref()
        .is_some_and(|u| u.contains("play.google.com"))));

    // Bleep sells lossless, iTunes does not — the tier split survives the fact
    // that a relation says nothing about format.
    let bleep = offers.iter().find(|o| o.vendor == Vendor::Bleep).unwrap();
    let itunes = offers.iter().find(|o| o.vendor == Vendor::ITunes).unwrap();
    assert_eq!(tier_of(bleep), Tier::A);
    assert_eq!(tier_of(itunes), Tier::B);

    // All of them are marked as an index rather than a check, and none claims a
    // price or a bit depth it cannot know.
    for offer in &offers {
        assert_eq!(offer.provenance, Provenance::Indexed, "{}", offer.vendor);
        assert!(offer.price.is_none(), "{} invented a price", offer.vendor);
        assert!(offer.url.is_some());
    }
    assert!(bleep.delivery.bit_depth.is_none(), "invented a bit depth");
}

#[test]
fn a_pressing_with_no_shop_links_is_empty_rather_than_a_fault() {
    // The 2000 CD. Nobody sells a CD as a download, so no editor added a link —
    // which is an answer, not a broken parse. Sleeve asks the digital sibling
    // for the same album instead.
    assert_eq!(
        musicbrainz::parse_purchase_links(&fixture("musicbrainz/release-urls-none.json")),
        Outcome::Empty
    );
}

#[test]
fn a_release_and_a_release_group_yield_different_kinds_of_discogs_link() {
    use sleeve::model::source::musicbrainz::DiscogsLink;

    // The two are not interchangeable and mixing them up is a silent 404:
    // Discogs master 21501 is Kid A, and Discogs *release* 21501 does not exist.
    // A release relation gives a release id, a release-group relation gives a
    // master id, and each needs a different endpoint.
    assert_eq!(
        musicbrainz::parse_discogs_link(&fixture("musicbrainz/release-urls-kid-a.json")),
        Some(DiscogsLink::Release(35429287)),
    );
    assert_eq!(
        musicbrainz::parse_discogs_link(&fixture("musicbrainz/release-group-urls-kid-a.json")),
        Some(DiscogsLink::Master(21501)),
    );
}

#[test]
fn an_indexed_link_and_a_live_check_of_one_shop_become_one_row() {
    use sleeve::model::offer::merge;

    let mut offers =
        musicbrainz::parse_purchase_links(&fixture("musicbrainz/release-urls-kid-a.json"))
            .found()
            .expect("indexed offers");

    // Bandcamp's own API, for the same album: a real price and a refusal to sell.
    let details = bandcamp::parse_details(&fixture("bandcamp/details-kid-a.json"), None)
        .found()
        .expect("details");
    assert!(!details.purchasable);
    offers.extend(details.to_offer());

    // The live "no" beats the indexed "yes", so Bandcamp leaves entirely rather
    // than appearing as a top-ranked offer nobody can buy.
    let merged = merge(offers, &[Vendor::Bandcamp]);
    assert!(!merged.iter().any(|offer| offer.vendor == Vendor::Bandcamp));
    // The shops nobody checked survive on their links.
    assert!(merged.iter().any(|offer| offer.vendor == Vendor::Bleep));
}

#[test]
fn the_recorded_artist_lookup_yields_every_name_radiohead_go_by() {
    let Outcome::Found(names) =
        musicbrainz::parse_artist_aliases(&fixture("musicbrainz/artist-radiohead.json"))
    else {
        panic!("expected aliases");
    };
    assert!(names.iter().any(|name| name == "Radiohead"));
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(sorted, names, "aliases must come back sorted and deduped");
}

// -- iTunes ------------------------------------------------------------------

#[test]
fn the_recorded_itunes_search_prices_the_album_as_a_lossy_purchase() {
    let Outcome::Found(albums) = itunes::parse_albums(&fixture("itunes/search-kid-a.json")) else {
        panic!("expected albums");
    };
    let kid_a = albums
        .iter()
        .find(|album| album.title == "Kid A")
        .expect("Kid A in the results");

    assert_eq!(kid_a.price, Some((7.99, "GBP".into())));
    assert_eq!(kid_a.artist, "Radiohead");
    assert!(kid_a.artwork_100.is_some());

    let offer = kid_a.to_offer();
    assert_eq!(tier_of(&offer), Tier::B);
    assert_eq!(offer.acquisition, Acquisition::Purchase);
    assert!(!offer.delivery.lossless);
}

#[test]
fn the_recorded_itunes_artwork_url_resizes() {
    let Outcome::Found(albums) = itunes::parse_albums(&fixture("itunes/search-kid-a.json")) else {
        panic!("expected albums");
    };
    let url = albums[0].artwork_100.as_ref().expect("artwork");
    let large = itunes::artwork_at(url, 600);
    assert!(large.contains("600x600"), "{large}");
    assert!(large.starts_with("https://"));
}

// -- Odesli ------------------------------------------------------------------

#[test]
fn the_recorded_odesli_response_becomes_streaming_offers_only() {
    let Outcome::Found(catalogue) =
        odesli::parse_catalogue(&fixture("odesli/links-kid-a.json"), true)
    else {
        panic!("expected a catalogue");
    };

    let vendors: Vec<Vendor> = catalogue.links.iter().map(|(vendor, _)| *vendor).collect();
    assert!(vendors.contains(&Vendor::AppleMusic));
    assert!(vendors.contains(&Vendor::Tidal));
    assert!(vendors.contains(&Vendor::Deezer));

    // The response also lists Pandora, Anghami, Napster, Yandex, Boomplay, the
    // iTunes store, the Amazon store and Bandcamp. None of those is a
    // subscription service this ranks, and the two stores have no price on
    // them — a purchase offer with no price is worse than no offer.
    for offer in catalogue.to_offers() {
        assert_eq!(
            offer.acquisition,
            Acquisition::Subscription,
            "{} came through as a purchase",
            offer.vendor
        );
        assert!(tier_of(&offer) >= Tier::C);
    }
}

// -- Bandcamp ----------------------------------------------------------------

#[test]
fn the_recorded_bandcamp_search_picks_radiohead_over_four_cover_versions() {
    // What Bandcamp really returns for "Radiohead Kid A", in order: a Halifax
    // Music Co-op page titled "Radiohead - Kid A", a string quartet recital, two
    // chiptune arrangements, and only then Radiohead's own listing. Four of
    // those five have the word "Radiohead" in the *title*, so matching on title
    // alone buys a cover version.
    let Outcome::Found(hits) = bandcamp::parse_search(&fixture("bandcamp/search-kid-a.json"))
    else {
        panic!("expected hits");
    };
    assert!(
        hits.len() >= 5,
        "the fixture should hold the cover versions too"
    );

    let best = bandcamp::best_hit(&hits, "Radiohead", "Kid A").expect("Radiohead's own listing");
    assert_eq!(best.title, "Kid A");
    assert_eq!(best.artist, "Radiohead");
}

#[test]
fn a_cover_version_is_rejected_when_its_artist_is_asked_for_and_absent() {
    let Outcome::Found(hits) = bandcamp::parse_search(&fixture("bandcamp/search-kid-a.json"))
    else {
        panic!("expected hits");
    };
    // Nobody in this response is Aphex Twin, so nothing should come back — the
    // title floor alone would happily hand over one of the Kid A covers.
    assert!(bandcamp::best_hit(&hits, "Aphex Twin", "Kid A").is_none());

    // And the co-op's own recording is still findable when it is what was asked
    // for. The floor rejects wrong artists, not unusual ones.
    assert!(bandcamp::best_hit(&hits, "Halifax Music Co-op", "Radiohead - Kid A").is_some());
}

#[test]
fn every_recorded_bandcamp_url_is_repaired() {
    let Outcome::Found(hits) = bandcamp::parse_search(&fixture("bandcamp/search-kid-a.json"))
    else {
        panic!("expected hits");
    };
    for hit in &hits {
        assert_eq!(
            hit.url.matches("https://").count(),
            1,
            "doubled URL survived: {}",
            hit.url
        );
    }
}

#[test]
fn the_recorded_bandcamp_details_refuse_to_sell_and_so_produce_no_offer() {
    // Radiohead's own page, a real 9.99 USD price, and `is_purchasable: false`.
    // Reading the price and skipping the flag would put a tier-A offer that does
    // not exist at the top of the ranking.
    let Outcome::Found(details) =
        bandcamp::parse_details(&fixture("bandcamp/details-kid-a.json"), None)
    else {
        panic!("expected details");
    };
    assert_eq!(details.title, "Kid A");
    assert_eq!(details.artist.as_deref(), Some("Radiohead"));
    assert_eq!(details.price, Some((9.99, "USD".into())));
    assert!(!details.purchasable);
    assert_eq!(details.to_offer(), None);
}

// -- Qobuz -------------------------------------------------------------------

#[test]
fn the_recorded_qobuz_player_gives_up_its_app_id() {
    let Outcome::Found(path) = qobuz::credentials::parse_bundle_path(&fixture("qobuz/player.html"))
    else {
        panic!("expected a bundle path");
    };
    assert!(path.starts_with("/resources/"));
    assert!(path.ends_with("bundle.js"));

    // A slice of the real minified bundle, which spells it `appId:` rather than
    // the `app_id:` every third-party client documents.
    let Outcome::Found(id) = qobuz::credentials::parse_app_id(&fixture("qobuz/bundle-slice.js"))
    else {
        panic!("expected an app id");
    };
    assert!(id.chars().all(|c| c.is_ascii_digit()), "{id}");
    assert!(id.len() >= 6);
}

#[test]
fn the_recorded_qobuz_refusal_points_at_the_config_file() {
    // The live API's answer to an app id with no user token. It must read as
    // "you have not configured this" rather than "Qobuz is broken", because the
    // two send a person to entirely different places.
    let outcome = qobuz::parse_search(&fixture("qobuz/search-kid-a.json"));
    let Outcome::Unusable(Reason::NotConfigured(message)) = outcome else {
        panic!("expected an unconfigured outcome, got {outcome:?}");
    };
    assert!(message.to_lowercase().contains("authentication"));
}

// -- Dynamic Range Database --------------------------------------------------

#[test]
fn a_rate_limited_dynamic_range_page_is_stale_and_never_a_measurement() {
    // What the database actually returned on the first request of the day. It is
    // an HTTP 429 with an HTML body and no table, and it must not be read as
    // "this album has no measurements" — that is a real answer with a real
    // scoring consequence.
    let body = b"<html><head><title>429 Too Many Requests</title></head><body>\
                 <h1>Too Many Requests</h1></body></html>";
    assert!(matches!(dynamic_range::parse(body), Outcome::Stale(_)));
}

// -- the whole pipeline ------------------------------------------------------

#[test]
fn the_recorded_sources_together_rank_the_way_the_principles_require() {
    use sleeve::model::score::rank;
    use sleeve::model::verdict::Verdict;
    use sleeve::model::weights::Weights;

    let mut offers = Vec::new();

    let Outcome::Found(albums) = itunes::parse_albums(&fixture("itunes/search-kid-a.json")) else {
        panic!("itunes");
    };
    offers.push(
        albums
            .iter()
            .find(|album| album.title == "Kid A")
            .unwrap()
            .to_offer(),
    );

    let Outcome::Found(catalogue) =
        odesli::parse_catalogue(&fixture("odesli/links-kid-a.json"), true)
    else {
        panic!("odesli");
    };
    offers.extend(catalogue.to_offers());

    // Qobuz was never configured — a standing choice, not a fact about this
    // album — and the Dynamic Range DB rate-limited, which is. Only the second
    // should reach the person.
    let gaps = vec![
        (
            sleeve::model::source::SourceId::QobuzStore,
            Reason::NotConfigured("no qobuz_user_token in config.toml".into()),
        ),
        (
            sleeve::model::source::SourceId::DynamicRange,
            Reason::RateLimited,
        ),
    ];

    let verdict = Verdict::assemble(None, rank(&offers, &Weights::default()), gaps);

    // The lossy purchase leads every lossless stream, which is the whole point.
    let best = verdict.ranked.best().expect("a best offer");
    assert_eq!(best.offer.vendor, Vendor::ITunes);
    assert_eq!(best.tier, Tier::B);
    for scored in verdict.ranked.ranked.iter().skip(1) {
        assert!(
            scored.tier >= Tier::C,
            "{} outranked the purchase",
            scored.offer.vendor
        );
    }

    // The real failure is named; the unset key is not.
    assert!(verdict
        .recommendation
        .contains("Not checked: Dynamic Range DB"));
    assert!(!verdict.recommendation.contains("Qobuz"));
    assert_eq!(verdict.unchecked.len(), 1);
}

#[test]
fn a_release_whose_only_link_is_discogs_is_empty_and_not_a_gap() {
    // Project 86's Truthless Heroes: one `discogs` relation, no shops. Nothing
    // was sold as a download in 2002, so there is nothing for an editor to have
    // linked. That is an answer, and reporting MusicBrainz as "not checked"
    // under every such album is both wrong and constant.
    let outcome =
        musicbrainz::parse_purchase_links(&fixture("musicbrainz/release-urls-discogs-only.json"));
    assert_eq!(outcome, Outcome::Empty, "{outcome:?}");
    assert_eq!(outcome.gap(), None, "reported as a gap");

    // The Discogs link on the same body is still picked up.
    assert_eq!(
        musicbrainz::parse_discogs_link(&fixture("musicbrainz/release-urls-discogs-only.json")),
        Some(sleeve::model::source::musicbrainz::DiscogsLink::Release(
            3546352
        )),
    );
}
