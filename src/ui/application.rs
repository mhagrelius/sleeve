//! `SleeveApplication`: the only object that owns state or asks anything.
//!
//! Every widget in `ui/` reports what a person did and waits. This file is where
//! that becomes a request, a ranking, and a page. Having one such place is what
//! keeps the widget tree free of `RefCell`s pointing at each other.
//!
//! The shape of a lookup: pick a release, then fire every source at once and
//! render each answer as it lands. Nothing waits for the slowest — the ranking
//! is recomputed and redrawn on every arrival, so Bandcamp appearing four
//! seconds after iTunes reorders the list in front of you rather than delaying
//! all of it. A source that never answers leaves a row in "not checked" and
//! costs nothing else.

use std::cell::{Cell, OnceCell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::gio;
use gtk::glib;

use super::art::Covers;
use super::http::Http;
use super::load_stylesheet;
use super::window::SleeveWindow;
use crate::model::album::{Mbid, Release, ReleaseGroup};
use crate::model::cache::{Cache, Kind};
use crate::model::candidate::Matches;
use crate::model::offer::{Offer, Vendor};
use crate::model::query::Query;
use crate::model::score::rank;
use crate::model::settings::{ConfigProblem, Settings};
use crate::model::source::musicbrainz::DiscogsLink;
use crate::model::source::{
    bandcamp, discogs, dynamic_range, itunes, musicbrainz, odesli, qobuz, Outcome, Reason, SourceId,
};
use crate::model::verdict::Verdict;
use crate::APP_ID;

/// One in-progress lookup.
///
/// Guarded by the application's generation counter rather than by anything here:
/// a callback from a lookup the person has already navigated away from finds a
/// stale generation and drops its result instead of writing it into the page
/// that replaced it.
#[derive(Default)]
pub struct Lookup {
    release: Option<Release>,
    offers: Vec<Offer>,
    unchecked: Vec<(SourceId, Reason)>,
    /// Vendors a live source asked and got a no from.
    ///
    /// These suppress MusicBrainz's indexed purchase links for the same shop: an
    /// editor's link says a record was on sale once, and a shop's own API says
    /// whether it is now.
    checked_absent: Vec<Vendor>,
}

impl Lookup {
    /// Record how a source got on, replacing whatever it said before.
    ///
    /// A success clears an earlier failure rather than leaving it standing. A
    /// source can be asked more than once in a lookup — the purchase links are
    /// fetched for a chosen pressing and again for its digital sibling — and
    /// without this the first answer sticks, so one leg failing marks the whole
    /// source as not checked even after the other leg brings the data back.
    fn record(&mut self, source: SourceId, outcome_gap: Option<Reason>) {
        self.unchecked.retain(|(id, _)| *id != source);
        if let Some(reason) = outcome_gap {
            self.unchecked.push((source, reason));
        }
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SleeveApplication {
        pub settings: RefCell<Settings>,
        pub problem: RefCell<Option<ConfigProblem>>,
        pub cache: OnceCell<Rc<Cache>>,
        pub http: OnceCell<Http>,
        pub covers: OnceCell<Covers>,
        pub window: RefCell<Option<SleeveWindow>>,
        pub lookup: RefCell<Lookup>,
        pub generation: Cell<u64>,
        /// The release group currently drilled into.
        pub group: RefCell<Option<ReleaseGroup>>,
        /// The releases of the group currently drilled into.
        pub releases: RefCell<Vec<Release>>,
        pub matches: RefCell<Matches>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SleeveApplication {
        const NAME: &'static str = "SleeveApplication";
        type Type = super::SleeveApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for SleeveApplication {}

    impl ApplicationImpl for SleeveApplication {
        fn startup(&self) {
            // Chain up first: the toolkit initialises in the parent handler, and
            // anything touching GTK before it is undefined.
            self.parent_startup();
            let app = self.obj();

            if let Some(display) = gtk::gdk::Display::default() {
                load_stylesheet(&display);
            }
            app.load_settings();
            app.open_cache();
            app.install_actions();
        }

        fn activate(&self) {
            let app = self.obj();
            app.window().present();
        }

        fn shutdown(&self) {
            let app = self.obj();
            // Housekeeping on the way out rather than on the way in: a first
            // paint should not wait on a cache sweep.
            if let Some(cache) = app.imp().cache.get() {
                let _ = cache.purge_expired();
                let _ = cache.sweep_files(&super::super::art::directory(&app.cache_dir()));
            }
            self.parent_shutdown();
        }
    }

    impl GtkApplicationImpl for SleeveApplication {}
    impl AdwApplicationImpl for SleeveApplication {}
}

glib::wrapper! {
    pub struct SleeveApplication(ObjectSubclass<imp::SleeveApplication>)
        @extends adw::Application, gtk::Application, gio::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl Default for SleeveApplication {
    fn default() -> Self {
        Self::new()
    }
}

impl SleeveApplication {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("application-id", APP_ID)
            .property("flags", gio::ApplicationFlags::default())
            .build()
    }

    // -- directories and startup --------------------------------------------

    fn config_dir(&self) -> PathBuf {
        glib::user_config_dir().join("sleeve")
    }

    fn cache_dir(&self) -> PathBuf {
        glib::user_cache_dir().join("sleeve")
    }

    fn load_settings(&self) {
        let dir = self.config_dir();
        let _ = std::fs::create_dir_all(&dir);

        // Write the annotated template on first run, so there is something to
        // edit rather than a blank file and a manual to find.
        let path = Settings::path_in(&dir);
        if !path.exists() {
            let _ = std::fs::write(&path, Settings::template());
        }

        let (settings, problem) = Settings::load(&dir);
        *self.imp().settings.borrow_mut() = settings;
        *self.imp().problem.borrow_mut() = problem;
    }

    fn open_cache(&self) {
        let cache = Cache::open(&self.cache_dir().join("cache.sqlite"))
            .or_else(|_| Cache::in_memory())
            .expect("a cache");
        let cache = Rc::new(cache);
        let http = Http::new(Rc::clone(&cache));
        let covers = Covers::new(&self.cache_dir(), http.clone(), Rc::clone(&cache));

        let _ = self.imp().cache.set(cache);
        let _ = self.imp().http.set(http);
        let _ = self.imp().covers.set(covers);
    }

    fn install_actions(&self) {
        let about = gio::SimpleAction::new("about", None);
        about.connect_activate(glib::clone!(
            #[weak(rename_to = app)]
            self,
            move |_, _| app.show_about()
        ));
        self.add_action(&about);

        let quit = gio::SimpleAction::new("quit", None);
        quit.connect_activate(glib::clone!(
            #[weak(rename_to = app)]
            self,
            move |_, _| app.quit()
        ));
        self.add_action(&quit);
        self.set_accels_for_action("app.quit", &["<Control>q"]);
    }

    fn show_about(&self) {
        let dialog = adw::AboutDialog::builder()
            .application_name("Sleeve")
            .application_icon(APP_ID)
            .version(env!("CARGO_PKG_VERSION"))
            .developer_name("Matthew Hagrelius")
            .license_type(gtk::License::Gpl30)
            .comments("Where to buy an album, ranked best-first.")
            .build();
        if let Some(window) = self.imp().window.borrow().as_ref() {
            dialog.present(Some(window));
        }
    }

    fn http(&self) -> Http {
        self.imp().http.get().expect("http").clone()
    }

    fn cache(&self) -> Rc<Cache> {
        Rc::clone(self.imp().cache.get().expect("a cache"))
    }

    fn covers(&self) -> Covers {
        self.imp().covers.get().expect("covers").clone()
    }

    fn settings(&self) -> Settings {
        self.imp().settings.borrow().clone()
    }

    // -- the window ---------------------------------------------------------

    fn window(&self) -> SleeveWindow {
        if let Some(window) = self.imp().window.borrow().as_ref() {
            return window.clone();
        }

        let window: SleeveWindow = glib::Object::builder()
            .property("application", self)
            .build();

        let search = window.search();
        search.connect_search(glib::clone!(
            #[weak(rename_to = app)]
            self,
            move |query| app.search(query)
        ));
        search.connect_choose(glib::clone!(
            #[weak(rename_to = app)]
            self,
            move |mbid| app.choose_group(&mbid)
        ));

        window.editions().connect_choose(glib::clone!(
            #[weak(rename_to = app)]
            self,
            move |release| app.look_up(release)
        ));

        // Conditions the person can act on, said once and left up.
        if let Some(message) = self.startup_warning() {
            search.set_banner(Some(&message));
        }

        *self.imp().window.borrow_mut() = Some(window.clone());
        window
    }

    /// A condition the person needs to know about and can act on.
    ///
    /// Only a broken config qualifies. There is deliberately no nag about
    /// `contact` being unset: MusicBrainz asks for a way to make contact, and the
    /// project URL Sleeve falls back to is one — issues can be filed there. Since
    /// the fallback is compliant, the only thing the banner achieved was pressing
    /// people to put a personal email address into a header sent to eight third
    /// parties on every search, which is a worse default than the one it was
    /// complaining about.
    fn startup_warning(&self) -> Option<String> {
        match self.imp().problem.borrow().as_ref() {
            Some(ConfigProblem::Invalid(why)) => Some(format!(
                "config.toml has an error, so defaults are in use: {why}"
            )),
            Some(ConfigProblem::Unreadable(why)) => Some(format!(
                "config.toml could not be read, so defaults are in use: {why}"
            )),
            _ => None,
        }
    }

    // -- searching ----------------------------------------------------------

    fn search(&self, query: Query) {
        let window = self.window();
        let search_page = window.search();
        search_page.set_searching(true);

        let generation = self.bump();
        let settings = self.settings();
        let request =
            musicbrainz::search_release_groups(&query.artist, &query.album, &settings.user_agent());

        self.http().fetch(
            request,
            Kind::Metadata,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                move |outcome| {
                    if app.imp().generation.get() != generation {
                        return;
                    }
                    let groups = match outcome {
                        Outcome::Found(body) => match musicbrainz::parse_release_groups(&body) {
                            Outcome::Found(groups) => groups,
                            _ => Vec::new(),
                        },
                        _ => Vec::new(),
                    };

                    let matches = crate::model::search::rank_candidates(&query, &groups);
                    *app.imp().matches.borrow_mut() = matches.clone();
                    let window = app.window();
                    window.search().set_matches(&matches, Some(&app.covers()));

                    // Near misses need a second request against the leading
                    // artist, so they arrive after the matches rather than
                    // holding them up.
                    app.fetch_near_misses(query, matches, generation);
                }
            ),
        );
    }

    fn fetch_near_misses(&self, query: Query, matches: Matches, generation: u64) {
        let Some(artist) = matches
            .best()
            .and_then(|candidate| candidate.group.artist.mbid.clone())
        else {
            return;
        };

        let settings = self.settings();
        let request = musicbrainz::browse_artist_release_groups(&artist, &settings.user_agent());
        self.http().fetch(
            request,
            Kind::Metadata,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                move |outcome| {
                    if app.imp().generation.get() != generation {
                        return;
                    }
                    let Outcome::Found(body) = outcome else {
                        return;
                    };
                    let Outcome::Found(groups) = musicbrainz::parse_release_groups(&body) else {
                        return;
                    };

                    let mut matches = matches.clone();
                    crate::model::search::add_near_misses(&mut matches, &query, &groups);
                    *app.imp().matches.borrow_mut() = matches.clone();
                    app.window()
                        .search()
                        .set_matches(&matches, Some(&app.covers()));
                }
            ),
        );
    }

    // -- drilling into a release group --------------------------------------

    fn choose_group(&self, mbid: &Mbid) {
        let Some(group) = self
            .imp()
            .matches
            .borrow()
            .candidates
            .iter()
            .chain(
                self.imp()
                    .matches
                    .borrow()
                    .near_misses
                    .iter()
                    .map(|miss| &miss.candidate),
            )
            .find(|candidate| &candidate.group.mbid == mbid)
            .map(|candidate| candidate.group.clone())
        else {
            return;
        };

        let window = self.window();
        window.editions().set_loading(&group);
        window.show_editions();

        let generation = self.bump();
        *self.imp().group.borrow_mut() = Some(group.clone());

        let settings = self.settings();
        let request = musicbrainz::browse_releases(&group.mbid, &settings.user_agent());
        self.http().fetch(
            request,
            Kind::Metadata,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                move |outcome| {
                    if app.imp().generation.get() != generation {
                        return;
                    }
                    let releases = match outcome {
                        Outcome::Found(body) => musicbrainz::parse_releases(&group.mbid, &body)
                            .found()
                            .unwrap_or_default(),
                        _ => Vec::new(),
                    };
                    app.window()
                        .editions()
                        .set_releases(&group, &releases, Some(&app.covers()));
                }
            ),
        );
    }

    // -- the lookup ---------------------------------------------------------

    fn look_up(&self, release: Release) {
        let window = self.window();
        window.result().set_looking(&release.title);
        window.show_result();

        let generation = self.bump();
        *self.imp().lookup.borrow_mut() = Lookup {
            release: Some(release.clone()),
            offers: Vec::new(),
            unchecked: Vec::new(),
            checked_absent: Vec::new(),
        };
        self.redraw(generation);

        let settings = self.settings();
        let artist = release.artist.name.clone();
        let title = release.title.clone();

        self.fetch_purchase_links(&release, generation, &settings);
        // Shop links hang off the pressing, not the album: the 2000 CD has none
        // even though the album is sold as files everywhere. When a physical
        // issue was chosen, ask its digital sibling as well — otherwise picking
        // the CD hides every download shop.
        if release.is_physical_only() {
            if let Some(digital) = self.digital_sibling(&release) {
                self.fetch_purchase_links(&digital, generation, &settings);
            }
        }
        self.fetch_itunes(&artist, &title, generation, &settings);
        self.fetch_bandcamp(&artist, &title, generation);
        self.fetch_dynamic_range(&artist, &title, release.year(), generation);
        self.fetch_qobuz(&artist, &title, generation, &settings);
    }

    /// The digital issue of the same album, if the group has one.
    fn digital_sibling(&self, chosen: &Release) -> Option<Release> {
        self.imp()
            .releases
            .borrow()
            .iter()
            .find(|release| release.mbid != chosen.mbid && !release.is_physical_only())
            .cloned()
    }

    /// Recompute and redraw. Called on every source arrival.
    fn redraw(&self, generation: u64) {
        if self.imp().generation.get() != generation {
            return;
        }
        let lookup = self.imp().lookup.borrow();
        let settings = self.settings();
        // Two sources routinely describe one shop — MusicBrainz indexes a link,
        // the shop's own API prices it. Merge before ranking, or the list shows
        // Bandcamp twice.
        let offers = crate::model::offer::merge(lookup.offers.clone(), &lookup.checked_absent);
        let ranked = rank(&offers, &settings.weights);
        let verdict = Verdict::assemble(lookup.release.clone(), ranked, lookup.unchecked.clone());
        drop(lookup);

        self.window()
            .result()
            .set_verdict(&verdict, Some(&self.covers()));
    }

    /// Fold one source's answer into the lookup and redraw.
    fn absorb(&self, generation: u64, source: SourceId, gap: Option<Reason>, offers: Vec<Offer>) {
        self.absorb_checked(generation, source, gap, offers, &[]);
    }

    /// The same, plus vendors this source checked and found nothing at.
    fn absorb_checked(
        &self,
        generation: u64,
        source: SourceId,
        gap: Option<Reason>,
        offers: Vec<Offer>,
        absent: &[Vendor],
    ) {
        if self.imp().generation.get() != generation {
            return;
        }
        {
            let mut lookup = self.imp().lookup.borrow_mut();
            lookup.record(source, gap);
            lookup.offers.extend(offers);
            for vendor in absent {
                if !lookup.checked_absent.contains(vendor) {
                    lookup.checked_absent.push(*vendor);
                }
            }
        }
        self.redraw(generation);
    }

    fn bump(&self) -> u64 {
        let next = self.imp().generation.get() + 1;
        self.imp().generation.set(next);
        next
    }

    // -- individual sources -------------------------------------------------

    /// MusicBrainz's purchase links: every shop, no credentials.
    ///
    /// The request that makes this application useful out of the box. It reaches
    /// Bleep, Boomkat, Presto, Beatport and Juno — all of which refuse a plain
    /// HTTP client — and Qobuz, whose own API wants a paid account, without
    /// asking anyone to sign up for anything.
    ///
    /// The same response carries the Discogs master an editor matched by hand,
    /// so it also saves a fuzzy Discogs search that could land on a tribute
    /// record.
    fn fetch_purchase_links(&self, release: &Release, generation: u64, settings: &Settings) {
        let request = musicbrainz::release_urls(&release.mbid, &settings.user_agent());
        let (artist, title) = (release.artist.name.clone(), release.title.clone());
        let currency = settings.currency.clone();
        let token = settings.keys.discogs_token.trim().to_string();
        let agent = settings.user_agent();

        self.http().fetch(
            request,
            Kind::Metadata,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                move |outcome| {
                    let body = match outcome {
                        Outcome::Found(body) => body,
                        other => {
                            app.absorb(
                                generation,
                                SourceId::MusicBrainz,
                                other.map(|_| ()).gap(),
                                Vec::new(),
                            );
                            return;
                        }
                    };

                    let parsed = musicbrainz::parse_purchase_links(&body);
                    let offers = match &parsed {
                        Outcome::Found(offers) => offers.clone(),
                        _ => Vec::new(),
                    };
                    app.absorb(generation, SourceId::MusicBrainz, parsed.gap(), offers);

                    match musicbrainz::parse_discogs_link(&body) {
                        // An editor tied these two records together by hand. A
                        // release link is one pressing and can be priced now; a
                        // master is the work and needs its version list first.
                        Some(DiscogsLink::Release(id)) => {
                            app.fetch_discogs_release(id, &currency, &token, &agent, generation)
                        }
                        Some(DiscogsLink::Master(id)) => {
                            app.fetch_discogs_master(id, &currency, &token, &agent, generation)
                        }
                        // Nobody has. Fall back to a text search, which can land
                        // on a tribute album — hence the preference above.
                        None => {
                            let settings = app.settings();
                            app.fetch_discogs(&artist, &title, generation, &settings);
                        }
                    }
                }
            ),
        );
    }

    /// The cheapest copy of a specific Discogs master.
    fn fetch_discogs_master(
        &self,
        master: u64,
        currency: &str,
        token: &str,
        agent: &str,
        generation: u64,
    ) {
        let request = discogs::master_versions(master, token, agent);
        let (currency, token, agent) = (currency.to_string(), token.to_string(), agent.to_string());

        self.http().fetch(
            request,
            Kind::Metadata,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                move |outcome| {
                    let parsed = match outcome {
                        Outcome::Found(body) => discogs::parse_versions(&body),
                        other => other.map(|_| Vec::new()),
                    };
                    let Outcome::Found(versions) = &parsed else {
                        app.absorb(generation, SourceId::Discogs, parsed.gap(), Vec::new());
                        return;
                    };
                    let Some(version) = versions.first().cloned() else {
                        app.absorb(generation, SourceId::Discogs, None, Vec::new());
                        return;
                    };
                    app.fetch_discogs_release(version.id, &currency, &token, &agent, generation);
                }
            ),
        );
    }

    fn fetch_discogs_release(
        &self,
        id: u64,
        currency: &str,
        token: &str,
        agent: &str,
        generation: u64,
    ) {
        let request = discogs::release(id, currency, token, agent);
        self.http().fetch(
            request,
            Kind::Price,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                move |outcome| {
                    let parsed = match outcome {
                        Outcome::Found(body) => discogs::parse_release(&body),
                        other => other.map(|_| unreachable!()),
                    };
                    let offers = match &parsed {
                        Outcome::Found(listing) => listing.to_offer().into_iter().collect(),
                        _ => Vec::new(),
                    };
                    app.absorb(generation, SourceId::Discogs, parsed.gap(), offers);
                }
            ),
        );
    }

    fn fetch_itunes(&self, artist: &str, title: &str, generation: u64, settings: &Settings) {
        let request = itunes::search_albums(artist, title, &settings.locale);
        let spotify_lossless = settings.spotify_lossless;
        let locale = settings.locale.clone();

        self.http().fetch(
            request,
            Kind::Price,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                move |outcome| {
                    let parsed = match outcome {
                        Outcome::Found(body) => itunes::parse_albums(&body),
                        other => other.map(|_| Vec::new()),
                    };
                    let albums = match &parsed {
                        Outcome::Found(albums) => albums.clone(),
                        _ => Vec::new(),
                    };
                    let offers = albums
                        .first()
                        .map(|album| vec![album.to_offer()])
                        .unwrap_or_default();
                    app.absorb(generation, SourceId::ITunes, parsed.gap(), offers);

                    // Odesli is seeded from the iTunes collection id, so it can
                    // only run once this has landed. It is never called during a
                    // search — its budget is about ten requests a minute.
                    if let Some(album) = albums.first() {
                        app.fetch_odesli(
                            album.collection_id,
                            &locale,
                            spotify_lossless,
                            generation,
                        );
                    }
                }
            ),
        );
    }

    fn fetch_odesli(
        &self,
        collection_id: u64,
        locale: &str,
        spotify_lossless: bool,
        generation: u64,
    ) {
        let request = odesli::links_for_itunes_album(collection_id, locale);
        self.http().fetch(
            request,
            Kind::Metadata,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                move |outcome| {
                    let parsed = match outcome {
                        Outcome::Found(body) => odesli::parse_catalogue(&body, spotify_lossless),
                        other => other.map(|_| Default::default()),
                    };
                    let offers = match &parsed {
                        Outcome::Found(catalogue) => catalogue.to_offers(),
                        _ => Vec::new(),
                    };
                    app.absorb(generation, SourceId::Odesli, parsed.gap(), offers);
                }
            ),
        );
    }

    fn fetch_bandcamp(&self, artist: &str, title: &str, generation: u64) {
        let request = bandcamp::search(&format!("{artist} {title}"));
        let (artist, title) = (artist.to_string(), title.to_string());

        self.http().fetch(
            request,
            Kind::Metadata,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                move |outcome| {
                    let parsed = match outcome {
                        Outcome::Found(body) => bandcamp::parse_search(&body),
                        other => other.map(|_| Vec::new()),
                    };
                    let hits = match &parsed {
                        Outcome::Found(hits) => hits.clone(),
                        _ => Vec::new(),
                    };

                    let best = bandcamp::best_hit(&hits, &artist, &title);
                    let Some(hit) = best else {
                        app.absorb(generation, SourceId::Bandcamp, parsed.gap(), Vec::new());
                        return;
                    };
                    app.fetch_bandcamp_details(hit.clone(), generation);
                }
            ),
        );
    }

    fn fetch_bandcamp_details(&self, hit: bandcamp::Hit, generation: u64) {
        let request = bandcamp::album_details(hit.band_id, hit.album_id);
        let url = hit.url.clone();

        self.http().fetch(
            request,
            Kind::Price,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                move |outcome| {
                    let parsed = match outcome {
                        Outcome::Found(body) => bandcamp::parse_details(&body, Some(url.clone())),
                        other => other.map(|_| unreachable!()),
                    };
                    let offers: Vec<Offer> = match &parsed {
                        // `is_purchasable` decides this, not the presence of a
                        // price — a Bandcamp page can show a price on an album
                        // it will not sell.
                        Outcome::Found(details) => details.to_offer().into_iter().collect(),
                        _ => Vec::new(),
                    };
                    // Answered, and the answer was no. That has to outrank
                    // MusicBrainz's indexed Bandcamp link for the same album,
                    // which is exactly the Kid A case.
                    let absent: &[Vendor] = match &parsed {
                        Outcome::Found(details) if !details.purchasable => &[Vendor::Bandcamp],
                        _ => &[],
                    };
                    app.absorb_checked(
                        generation,
                        SourceId::Bandcamp,
                        parsed.gap(),
                        offers,
                        absent,
                    );
                }
            ),
        );
    }

    fn fetch_discogs(&self, artist: &str, title: &str, generation: u64, settings: &Settings) {
        // No token needed. Discogs answers unauthenticated at 25 requests a
        // minute; a token raises that to 60 and changes nothing else.
        let token = settings.keys.discogs_token.trim().to_string();
        let request = discogs::search_masters(artist, title, &token, &settings.user_agent());
        let currency = settings.currency.clone();
        let agent = settings.user_agent();

        self.http().fetch(
            request,
            Kind::Metadata,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                move |outcome| {
                    let parsed = match outcome {
                        Outcome::Found(body) => discogs::parse_masters(&body),
                        other => other.map(|_| Vec::new()),
                    };
                    let Outcome::Found(masters) = &parsed else {
                        app.absorb(generation, SourceId::Discogs, parsed.gap(), Vec::new());
                        return;
                    };
                    let Some(master) = masters.first().cloned() else {
                        app.absorb(generation, SourceId::Discogs, None, Vec::new());
                        return;
                    };

                    let request = discogs::release(master.id, &currency, &token, &agent);
                    app.http().fetch(
                        request,
                        Kind::Price,
                        glib::clone!(
                            #[weak(rename_to = app)]
                            app,
                            move |outcome| {
                                let parsed = match outcome {
                                    Outcome::Found(body) => discogs::parse_release(&body),
                                    other => other.map(|_| unreachable!()),
                                };
                                let offers = match &parsed {
                                    Outcome::Found(listing) => {
                                        listing.to_offer().into_iter().collect()
                                    }
                                    _ => Vec::new(),
                                };
                                app.absorb(generation, SourceId::Discogs, parsed.gap(), offers);
                            }
                        ),
                    );
                }
            ),
        );
    }

    /// The dynamic-range lookup, which decorates offers rather than adding any.
    fn fetch_dynamic_range(&self, artist: &str, title: &str, year: Option<i32>, generation: u64) {
        let request = dynamic_range::search(artist, title);
        self.http().fetch(
            request,
            Kind::Metadata,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                move |outcome| {
                    if app.imp().generation.get() != generation {
                        return;
                    }
                    let parsed = match outcome {
                        Outcome::Found(body) => dynamic_range::parse(&body),
                        other => other.map(|_| Vec::new()),
                    };
                    let matched = match &parsed {
                        Outcome::Found(entries) => {
                            dynamic_range::best_match(entries, year).cloned()
                        }
                        _ => None,
                    };

                    {
                        let mut lookup = app.imp().lookup.borrow_mut();
                        lookup.record(SourceId::DynamicRange, parsed.gap());
                        if let Some(entry) = &matched {
                            // Only offers with no measurement of their own get
                            // this one, so a source that named its own edition
                            // keeps it.
                            for offer in &mut lookup.offers {
                                if offer.dynamic_range.is_none() {
                                    offer.dynamic_range = Some(crate::model::offer::DynamicRange {
                                        dr: entry.dr,
                                        matched: entry.describe(),
                                    });
                                }
                            }
                        }
                    }
                    app.redraw(generation);
                }
            ),
        );
    }

    fn fetch_qobuz(&self, artist: &str, title: &str, generation: u64, settings: &Settings) {
        // Their catalogue refuses an app id on its own. Without an account token
        // there is nothing to try, so this says so rather than spending two
        // requests learning it again.
        if settings.keys.qobuz_user_token.trim().is_empty() {
            self.absorb(
                generation,
                SourceId::QobuzStore,
                Some(Reason::NotConfigured(
                    "no qobuz_user_token in config.toml".into(),
                )),
                Vec::new(),
            );
            return;
        }

        let configured = settings.keys.qobuz_app_id.trim().to_string();
        if !configured.is_empty() {
            self.qobuz_search(artist, title, &configured, generation, settings);
            return;
        }

        // Cached from a previous run's read of the web player. The id rotates
        // when Qobuz redeploys, so this expires with everything else.
        if let Ok(Some(bytes)) = self.cache().body(QOBUZ_APP_ID_KEY) {
            if let Ok(id) = String::from_utf8(bytes) {
                self.qobuz_search(artist, title, &id, generation, settings);
                return;
            }
        }

        let (artist, title) = (artist.to_string(), title.to_string());
        let settings = settings.clone();
        self.http().fetch(
            qobuz::credentials::player_page(),
            Kind::Metadata,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                move |outcome| {
                    let Outcome::Found(body) = outcome else {
                        app.absorb(
                            generation,
                            SourceId::QobuzStore,
                            Some(Reason::Network("could not reach the Qobuz player".into())),
                            Vec::new(),
                        );
                        return;
                    };
                    let Outcome::Found(path) = qobuz::credentials::parse_bundle_path(&body) else {
                        app.absorb(
                            generation,
                            SourceId::QobuzStore,
                            Some(Reason::Malformed(
                                "the Qobuz player has changed shape".into(),
                            )),
                            Vec::new(),
                        );
                        return;
                    };

                    app.http().fetch(
                        qobuz::credentials::bundle(&path),
                        Kind::Metadata,
                        glib::clone!(
                            #[weak(rename_to = app)]
                            app,
                            move |outcome| {
                                let id = match outcome {
                                    Outcome::Found(body) => {
                                        qobuz::credentials::parse_app_id(&body).found()
                                    }
                                    _ => None,
                                };
                                let Some(id) = id else {
                                    app.absorb(
                                        generation,
                                        SourceId::QobuzStore,
                                        Some(Reason::Malformed(
                                            "no app id in the Qobuz player".into(),
                                        )),
                                        Vec::new(),
                                    );
                                    return;
                                };
                                let _ = app.cache().store_body(
                                    QOBUZ_APP_ID_KEY,
                                    Kind::Metadata,
                                    id.as_bytes(),
                                );
                                app.qobuz_search(&artist, &title, &id, generation, &settings);
                            }
                        ),
                    );
                }
            ),
        );
    }

    fn qobuz_search(
        &self,
        artist: &str,
        title: &str,
        app_id: &str,
        generation: u64,
        settings: &Settings,
    ) {
        let locale = format!("{}-{}", settings.locale.to_lowercase(), settings.locale);
        let request = qobuz::search_albums(
            &format!("{artist} {title}"),
            app_id,
            settings.keys.qobuz_user_token.trim(),
            &locale,
        );
        let title = title.to_string();

        self.http().fetch(
            request,
            Kind::Price,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                move |outcome| {
                    let parsed = match outcome {
                        Outcome::Found(body) => qobuz::parse_search(&body),
                        other => other.map(|_| Vec::new()),
                    };
                    let offers = match &parsed {
                        Outcome::Found(albums) => albums
                            .iter()
                            .max_by(|a, b| {
                                similarity(&title, &a.title)
                                    .partial_cmp(&similarity(&title, &b.title))
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            })
                            // One album, up to two offers: the store sells files
                            // and the subscription streams them, and they are
                            // not the same product or the same tier.
                            .map(|album| album.to_offers())
                            .unwrap_or_default(),
                        _ => Vec::new(),
                    };
                    app.absorb(generation, SourceId::QobuzStore, parsed.gap(), offers);
                }
            ),
        );
    }
}

/// Where the cached Qobuz app id lives.
///
/// Not a URL, but the responses table is keyed by string and this wants exactly
/// the same expiry behaviour as everything else in it.
const QOBUZ_APP_ID_KEY: &str = "sleeve:qobuz-app-id";

/// Title closeness, for picking which of a shop's hits is the album asked for.
fn similarity(wanted: &str, candidate: &str) -> f64 {
    use crate::model::query::fold;
    rapidfuzz::fuzz::ratio(fold(wanted).chars(), fold(candidate).chars())
}
