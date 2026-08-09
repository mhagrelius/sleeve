//! Cover art: fetched lazily, cached on disk, never waited for.
//!
//! A cover is worth having and not worth blocking on. A row shows a placeholder
//! the moment it exists and swaps in the picture whenever it arrives; if none
//! ever arrives the placeholder is the answer. Nothing here reports an error,
//! because there is no error here a person could act on.
//!
//! Files live under `~/.cache/sleeve/covers` with only their paths in SQLite —
//! a database full of JPEGs is one you cannot inspect or copy. Misses are
//! recorded too, so an album with no art anywhere stops re-running the whole
//! fallback chain on every search.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;

use super::http::Http;
use crate::model::cache::{file_name_for, Cache, Kind};
use crate::model::source::art::{Attempt, Size};
use crate::model::source::Outcome;

/// Fetches, caches and hands out textures.
///
/// One per application, so that twenty rows showing the same sleeve decode it
/// once.
#[derive(Clone)]
pub struct Covers {
    directory: PathBuf,
    http: Http,
    cache: Rc<Cache>,
    textures: Rc<RefCell<HashMap<String, gdk::Texture>>>,
    in_flight: Rc<RefCell<Vec<String>>>,
}

impl Covers {
    pub fn new(cache_dir: &Path, http: Http, cache: Rc<Cache>) -> Self {
        let directory = cache_dir.join("covers");
        let _ = std::fs::create_dir_all(&directory);
        Covers {
            directory,
            http,
            cache,
            textures: Rc::new(RefCell::new(HashMap::new())),
            in_flight: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// A texture that is already decoded, without fetching anything.
    pub fn peek(&self, key: &str) -> Option<gdk::Texture> {
        self.textures.borrow().get(key).cloned()
    }

    /// Ask for a cover, calling `deliver` now or later, at most once.
    ///
    /// `key` identifies the album — an MBID where there is one. `chain` is the
    /// fallback order from [`crate::model::source::art::chain`]; it is walked in
    /// order and the first image to arrive wins.
    pub fn load<F>(&self, key: &str, chain: Vec<Attempt>, deliver: F)
    where
        F: Fn(gdk::Texture) + 'static,
    {
        if let Some(texture) = self.peek(key) {
            deliver(texture);
            return;
        }

        match self.cache.cover(key) {
            // Looked before and found nothing anywhere. Do not look again.
            Ok(Some(None)) => return,
            Ok(Some(Some(path))) => {
                if let Some(texture) = self.decode(key, &path) {
                    deliver(texture);
                    return;
                }
                // The row points at a file that is gone — fall through and
                // refetch rather than showing a placeholder forever.
            }
            _ => {}
        }

        if self.in_flight.borrow().iter().any(|other| other == key) {
            return;
        }
        self.in_flight.borrow_mut().push(key.to_string());
        self.walk(key.to_string(), chain, Rc::new(deliver));
    }

    /// Try each source in turn until one produces an image.
    fn walk(&self, key: String, mut chain: Vec<Attempt>, deliver: Rc<dyn Fn(gdk::Texture)>) {
        if chain.is_empty() {
            // Every source has been asked and none had it. Remember that.
            let _ = self.cache.store_cover_miss(&key);
            self.forget(&key);
            return;
        }

        let attempt = chain.remove(0);
        let covers = self.clone();

        match attempt {
            Attempt::Direct(url) => {
                let request =
                    crate::model::source::Request::get(crate::model::source::SourceId::Deezer, url);
                self.http.fetch(request, Kind::Metadata, move |outcome| {
                    covers.received(key, chain, deliver, outcome);
                });
            }
            Attempt::CoverArtArchive(request) | Attempt::Deezer(request) => {
                let is_deezer_search = request.url.contains("api.deezer.com/search");
                self.http.fetch(request, Kind::Metadata, move |outcome| {
                    if is_deezer_search {
                        // Deezer answers a search with JSON naming a picture,
                        // not with the picture, so this leg costs a second hop.
                        let next = match outcome {
                            Outcome::Found(body) => {
                                match crate::model::source::art::parse_deezer_cover(
                                    &body,
                                    Size::Full,
                                ) {
                                    Outcome::Found(url) => Some(url),
                                    _ => None,
                                }
                            }
                            _ => None,
                        };
                        match next {
                            Some(url) => {
                                let mut chain = chain;
                                chain.insert(0, Attempt::Direct(url));
                                covers.walk(key, chain, deliver);
                            }
                            None => covers.walk(key, chain, deliver),
                        }
                        return;
                    }
                    covers.received(key, chain, deliver, outcome);
                });
            }
        }
    }

    fn received(
        &self,
        key: String,
        chain: Vec<Attempt>,
        deliver: Rc<dyn Fn(gdk::Texture)>,
        outcome: Outcome<Vec<u8>>,
    ) {
        let Outcome::Found(bytes) = outcome else {
            // A 404 from the archive is the normal case, not a fault. Next.
            self.walk(key, chain, deliver);
            return;
        };
        if bytes.is_empty() {
            self.walk(key, chain, deliver);
            return;
        }

        let path = self.directory.join(file_name_for(&key, "img"));
        if std::fs::write(&path, &bytes).is_err() {
            self.walk(key, chain, deliver);
            return;
        }

        match self.decode(&key, &path) {
            Some(texture) => {
                let _ = self.cache.store_cover(&key, &path);
                self.forget(&key);
                deliver(texture);
            }
            None => {
                // Bytes that are not a picture: an HTML error page served with a
                // 200, most often. Drop it and carry on down the chain.
                let _ = std::fs::remove_file(&path);
                self.walk(key, chain, deliver);
            }
        }
    }

    fn decode(&self, key: &str, path: &Path) -> Option<gdk::Texture> {
        let texture = gdk::Texture::from_filename(path).ok()?;
        self.textures
            .borrow_mut()
            .insert(key.to_string(), texture.clone());
        Some(texture)
    }

    fn forget(&self, key: &str) {
        self.in_flight.borrow_mut().retain(|other| other != key);
    }
}

/// A square frame that shows a sleeve, or a stand-in until there is one.
///
/// The picture is hidden until a texture lands, so a missing cover renders as a
/// dimmed symbolic icon rather than as an empty hole or an error. Content is
/// cropped to fill: a row of sleeves with grey bars down the sides reads as
/// broken, and album art is square nearly always.
pub fn sleeve(size: i32) -> (gtk::Widget, gtk::Picture) {
    let placeholder = gtk::Image::builder()
        .icon_name("media-optical-symbolic")
        .pixel_size((size / 2).clamp(16, 64))
        .build();
    placeholder.add_css_class("dimmed");

    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .visible(false)
        .build();

    let overlay = gtk::Overlay::builder()
        .width_request(size)
        .height_request(size)
        .overflow(gtk::Overflow::Hidden)
        .valign(gtk::Align::Center)
        .child(&placeholder)
        .build();
    overlay.add_overlay(&picture);
    overlay.add_css_class("sleeve");

    (overlay.upcast(), picture)
}

/// Show a texture in a frame built by [`sleeve`].
pub fn show(picture: &gtk::Picture, texture: &gdk::Texture) {
    picture.set_paintable(Some(texture));
    picture.set_visible(true);
}

/// Where the covers live, for the housekeeping sweep.
pub fn directory(cache_dir: &Path) -> PathBuf {
    cache_dir.join("covers")
}

/// Bind a cover to a picture, dropping the update if the row is gone by then.
///
/// A search that is retyped rebuilds the list while requests from the previous
/// one are still in the air. Without the weak reference those late arrivals
/// would keep dead widgets alive and paint sleeves into rows that no longer
/// mean anything.
pub fn bind(covers: &Covers, key: &str, chain: Vec<Attempt>, picture: &gtk::Picture) {
    let weak = picture.downgrade();
    covers.load(key, chain, move |texture| {
        if let Some(picture) = weak.upgrade() {
            show(&picture, &texture);
        }
    });
}
