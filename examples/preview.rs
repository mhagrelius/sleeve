//! Render the real widget tree to a PNG.
//!
//! Screenshotting a live GNOME Wayland session needs interactive consent, which
//! makes "does this look right?" hard to answer while iterating. This builds the
//! actual pages against made-up data and paints them offscreen instead, so a
//! design change can be looked at in one command.
//!
//! The states worth a picture are the ones that are awkward to reach on demand:
//! an ambiguous search, a ranking with two masters in it, a source that failed,
//! and an album nobody sells.
//!
//! ```sh
//! cargo run --example preview -- /tmp/preview
//! cargo run --example preview -- /tmp/preview dark
//! ```

use std::fs;

use adw::prelude::*;
use gtk::glib;

use sleeve::model::album::{ArtistCredit, Label, Mbid, PrimaryType, Release, ReleaseGroup};
use sleeve::model::candidate::{Candidate, Matches, NearMiss, NearMissKind};
use sleeve::model::offer::{Acquisition, Delivery, Friction, Offer, Vendor};
use sleeve::model::score::rank;
use sleeve::model::source::{Reason, SourceId};
use sleeve::model::verdict::Verdict;
use sleeve::model::weights::Weights;
use sleeve::ui::{EditionsPage, ResultPage, SearchPage};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "/tmp/preview".to_string());
    let dark = args.next().is_some_and(|scheme| scheme == "dark");

    gtk::init().expect("a display — run under xvfb-run if there is none");
    adw::init().expect("libadwaita");

    // An animating widget is a widget that is not finished being laid out.
    // Turning animations off makes a snapshot deterministic rather than a race
    // against a transition.
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_enable_animations(false);
    }

    adw::StyleManager::default().set_color_scheme(if dark {
        adw::ColorScheme::ForceDark
    } else {
        adw::ColorScheme::ForceLight
    });
    if let Some(display) = gtk::gdk::Display::default() {
        sleeve::ui::load_stylesheet(&display);
    }

    fs::create_dir_all(&out).expect("output directory");
    let suffix = if dark { "dark" } else { "light" };

    // A first run: nothing typed, and the banner that says the config needs a
    // contact address before MusicBrainz will talk to us.
    let empty = SearchPage::new();
    empty.set_banner(Some(
        "Sleeve is identifying itself to MusicBrainz by its project URL. Set `contact` in \
         config.toml so they reach you rather than blocking the application.",
    ));
    render(
        &empty.page,
        760,
        640,
        &format!("{out}/search-empty-{suffix}.png"),
    );

    // An ambiguous search: several plausible answers and a near-miss section.
    let search = SearchPage::new();
    search.set_matches(&ambiguous(), None);
    render(
        &search.page,
        760,
        860,
        &format!("{out}/search-ambiguous-{suffix}.png"),
    );

    // The drill-down, with named editions separated from plain issues.
    let editions = EditionsPage::new();
    let (group, releases) = pressings();
    editions.set_releases(&group, &releases, None);
    render(
        &editions.page,
        760,
        820,
        &format!("{out}/editions-{suffix}.png"),
    );

    // The answer, with everything worth showing at once: a tier-A winner, two
    // masters in circulation, a cheaper option that is not the best one, a
    // region-locked offer, and a source that could not be reached.
    let result = ResultPage::new();
    result.set_verdict(&full_verdict(), None);
    render(
        &result.page,
        760,
        1100,
        &format!("{out}/result-{suffix}.png"),
    );

    // The same page with the winner's arithmetic open. This is the claim the
    // whole application rests on — that a ranking can show its working — so it
    // gets a picture of its own.
    let expanded = ResultPage::new();
    expanded.set_verdict(&full_verdict(), None);
    expand_first_row(&expanded.page);
    render(
        &expanded.page,
        760,
        1300,
        &format!("{out}/result-expanded-{suffix}.png"),
    );

    // Tier E: no legitimate purchase path, stated plainly and linking nowhere.
    let nowhere = ResultPage::new();
    nowhere.set_verdict(&unavailable_verdict(), None);
    render(
        &nowhere.page,
        760,
        620,
        &format!("{out}/result-nowhere-{suffix}.png"),
    );

    println!("wrote previews to {out}");
}

// -- fabricated data ---------------------------------------------------------

fn group_named(title: &str, artist: &str, year: i32, comment: Option<&str>) -> ReleaseGroup {
    ReleaseGroup {
        mbid: Mbid::new(format!("{artist}-{title}")),
        title: title.into(),
        artist: ArtistCredit::new(artist),
        first_release_year: Some(year),
        primary_type: Some(PrimaryType::Album),
        secondary_types: Vec::new(),
        disambiguation: comment.map(str::to_string),
        release_count: 6,
    }
}

fn candidate(group: ReleaseGroup, blended: f64) -> Candidate {
    Candidate {
        group,
        search_score: blended,
        title_similarity: blended,
        artist_similarity: blended,
        blended,
    }
}

/// The case the near-miss section exists for: a soundtrack title that several
/// unrelated records share.
fn ambiguous() -> Matches {
    Matches {
        candidates: vec![
            candidate(
                group_named(
                    "Blade Runner",
                    "Vangelis",
                    1994,
                    Some("1994 soundtrack album"),
                ),
                0.91,
            ),
            candidate(
                group_named(
                    "Blade Runner",
                    "Vangelis",
                    2007,
                    Some("Esper Edition, 3 discs"),
                ),
                0.88,
            ),
            candidate(
                group_named(
                    "Blade Runner 2049",
                    "Hans Zimmer & Benjamin Wallfisch",
                    2017,
                    None,
                ),
                0.72,
            ),
        ],
        near_misses: vec![
            NearMiss {
                candidate: candidate(
                    group_named(
                        "Blade Runner Trilogy",
                        "Vangelis",
                        2007,
                        Some("25th anniversary"),
                    ),
                    0.66,
                ),
                kind: NearMissKind::SameArtistOtherRelease,
            },
            NearMiss {
                candidate: candidate(
                    group_named("Blade Runner", "The New American Orchestra", 1982, None),
                    0.61,
                ),
                kind: NearMissKind::OtherArtistSimilarTitle,
            },
        ],
    }
}

fn pressings() -> (ReleaseGroup, Vec<Release>) {
    let group = group_named("Blade Runner", "Vangelis", 1994, None);
    let base =
        |title: &str, date: &str, country: &str, formats: &[&str], comment: Option<&str>| Release {
            mbid: Mbid::new(format!("{title}-{date}")),
            group: group.mbid.clone(),
            title: title.into(),
            artist: ArtistCredit::new("Vangelis"),
            date: Some(date.into()),
            country: Some(country.into()),
            labels: vec![Label {
                name: "East West".into(),
                catalog_number: Some("4509-96574-2".into()),
            }],
            formats: formats.iter().map(|f| (*f).to_string()).collect(),
            track_count: 12,
            disambiguation: comment.map(str::to_string),
            packaging: None,
            status: Some("Official".into()),
            barcode: None,
        };

    let releases = vec![
        base("Blade Runner", "1994-06-10", "GB", &["CD"], None),
        base("Blade Runner", "1994", "US", &["Cassette"], None),
        base(
            "Blade Runner",
            "2007-12-03",
            "GB",
            &["CD", "CD", "CD"],
            Some("Esper Edition, 25th anniversary"),
        ),
        base(
            "Blade Runner",
            "2017-04-21",
            "XW",
            &["Digital Media"],
            Some("2017 remaster"),
        ),
    ];
    (group, releases)
}

fn full_verdict() -> Verdict {
    let release = pressings().1.remove(0);

    let offers = vec![
        Offer::new(
            Vendor::Bandcamp,
            Acquisition::Purchase,
            Delivery::lossless("FLAC", 24, 96_000),
        )
        .with_price(11.00, "GBP")
        .with_url("https://example.bandcamp.com/album/blade-runner")
        .with_edition("1994 original")
        .with_dynamic_range(13, "1994, CD, lossless"),
        Offer::new(
            Vendor::QobuzStore,
            Acquisition::Purchase,
            Delivery::lossless("FLAC", 24, 192_000),
        )
        .with_price(13.49, "GBP")
        .with_url("https://www.qobuz.com/gb-en/album/blade-runner")
        .with_edition("2017 remaster")
        .with_dynamic_range(6, "2017, WEB, lossless"),
        Offer::new(
            Vendor::PhysicalUsed,
            Acquisition::Purchase,
            Delivery::lossless("CD", 16, 44_100),
        )
        .with_price(4.50, "GBP")
        .with_url("https://www.discogs.com/release/12345")
        .with_friction(Friction::RequiresRipping)
        .with_edition("1994 pressing")
        .with_note("31 for sale on Discogs"),
        Offer::new(
            Vendor::ITunes,
            Acquisition::Purchase,
            Delivery::lossy("AAC 256"),
        )
        .with_price(7.99, "GBP")
        .with_url("https://music.apple.com/gb/album/blade-runner/1"),
        Offer::new(
            Vendor::Qobuz,
            Acquisition::Subscription,
            Delivery::lossless("FLAC", 24, 192_000),
        )
        .with_url("https://open.qobuz.com/album/x"),
        Offer::new(
            Vendor::Spotify,
            Acquisition::Subscription,
            Delivery::lossless("FLAC", 24, 44_100),
        )
        .with_url("https://open.spotify.com/album/x"),
        Offer::new(
            Vendor::Bleep,
            Acquisition::Purchase,
            Delivery::lossless("FLAC", 24, 96_000),
        )
        .with_price(9.99, "GBP")
        .with_friction(Friction::RegionLocked),
        // Two shops reached only through MusicBrainz's purchase links: no API
        // would talk to either, and neither carries a price. They still rank
        // where they belong, and the rows say where they came from.
        Offer::new(
            Vendor::Boomkat,
            Acquisition::Purchase,
            Delivery {
                lossless: true,
                codec: Some("FLAC".into()),
                ..Delivery::default()
            },
        )
        .indexed()
        .with_url("https://boomkat.com/products/blade-runner"),
        Offer::new(
            Vendor::JunoDownload,
            Acquisition::Purchase,
            Delivery {
                lossless: true,
                codec: Some("FLAC".into()),
                ..Delivery::default()
            },
        )
        .indexed()
        .with_url("https://www.junodownload.com/products/blade-runner/123456"),
    ];

    Verdict::assemble(
        Some(release),
        rank(&offers, &Weights::default()),
        // Only a genuine failure. An unset optional key is not reported when the
        // index already covers that shop, which is why there is no Qobuz line
        // here despite there being no Qobuz token.
        vec![(
            SourceId::DynamicRange,
            Reason::Malformed("check failed — rate limited".into()),
        )],
    )
}

fn unavailable_verdict() -> Verdict {
    let mut release = pressings().1.remove(0);
    release.title = "Music From the Body".into();
    release.disambiguation = Some("promotional issue".into());
    Verdict::assemble(Some(release), rank(&[], &Weights::default()), Vec::new())
}

// -- the harness -------------------------------------------------------------

/// Open the first expander in a page, so a picture can show what is inside one.
///
/// Walks the tree rather than reaching into the page, because the rows are built
/// inside `set_verdict` and exposing them just for a screenshot would put a hole
/// in the widget's interface for the sake of this file.
fn expand_first_row(widget: &impl IsA<gtk::Widget>) -> bool {
    let mut child = widget.as_ref().first_child();
    while let Some(current) = child {
        if let Ok(expander) = current.clone().downcast::<adw::ExpanderRow>() {
            expander.set_expanded(true);
            return true;
        }
        if expand_first_row(&current) {
            return true;
        }
        child = current.next_sibling();
    }
    false
}

/// Paint a page's contents and write them out.
///
/// The page's child is taken out and rendered on its own rather than the page
/// being put in a `NavigationView`: these are pictures of a layout, and a window
/// decoration and a back button around one read as a mistake.
fn render(page: &adw::NavigationPage, width: i32, height: i32, path: &str) {
    let Some(child) = page.child() else {
        eprintln!("{path}: the page has no content");
        return;
    };
    page.set_child(gtk::Widget::NONE);

    // The height is a floor, not a promise. A page that needs more room gets it
    // rather than being cropped, which is how a layout that overflows would
    // otherwise look fine in the picture.
    for factor in [1, 2, 3] {
        if try_render(&child, width, height * factor, path) {
            page.set_child(Some(&child));
            return;
        }
    }
    eprintln!("{path}: nothing was drawn, even with room to spare");
    page.set_child(Some(&child));
}

fn try_render(widget: &impl IsA<gtk::Widget>, width: i32, height: i32, path: &str) -> bool {
    let window = gtk::Window::builder()
        .default_width(width)
        .default_height(height)
        .child(widget)
        .build();
    window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    window.present();

    settle();
    let drawn = snapshot(
        &window,
        window.width().max(width),
        window.height().max(height),
        path,
    );

    // Take the widget back before the window goes, so the page can have it.
    window.set_child(gtk::Widget::NONE);
    window.destroy();
    drawn
}

/// Run the main loop until there is nothing left to lay out.
///
/// One drain is not enough: presenting a widget schedules work that schedules
/// more, so this pumps until it stops finding any, with a bound so a
/// misbehaving widget cannot hang the run.
fn settle() {
    let context = glib::MainContext::default();
    for _ in 0..100 {
        let mut worked = false;
        while context.iteration(false) {
            worked = true;
        }
        if !worked {
            break;
        }
    }
}

/// Paint a realised window into a PNG. Reports whether anything was drawn.
fn snapshot(window: &impl IsA<gtk::Widget>, width: i32, height: i32, path: &str) -> bool {
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(width), f64::from(height));

    let Some(node) = snapshot.to_node() else {
        return false;
    };
    let renderer = gtk::gsk::CairoRenderer::new();
    renderer
        .realize(gtk::gdk::Surface::NONE)
        .expect("a renderer");
    let texture = renderer.render_texture(&node, None);
    texture.save_to_png(path).expect("write the png");
    renderer.unrealize();
    true
}
