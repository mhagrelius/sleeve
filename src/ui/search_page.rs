//! The search page: type two things, get a list of plausible records.
//!
//! Candidates are never auto-picked, so this page's job is to make telling them
//! apart easy: sleeve, title, artist, year, type, and MusicBrainz's own
//! disambiguation comment, all on one line each.
//!
//! Near misses sit in their own group below the matches, with their own heading
//! and their own explanation of why each is there. Interleaving them would make
//! a confident answer look like a guess; hiding them would lose the thing that
//! most often turns out to be what was wanted.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use super::art::{bind, Covers};
use super::candidate_row;
use crate::model::album::Mbid;
use crate::model::candidate::{Confidence, Matches};
use crate::model::query::Query;
use crate::model::source::art::{chain, Size};

type Handler<T> = Rc<RefCell<Option<Box<dyn Fn(T)>>>>;

pub struct SearchPage {
    pub page: adw::NavigationPage,
    artist: adw::EntryRow,
    album: adw::EntryRow,
    stack: gtk::Stack,
    matches_group: adw::PreferencesGroup,
    misses_group: adw::PreferencesGroup,
    matches_list: gtk::ListBox,
    misses_list: gtk::ListBox,
    banner: adw::Banner,
    on_search: Handler<Query>,
    on_choose: Handler<Mbid>,
}

impl SearchPage {
    pub fn new() -> Rc<SearchPage> {
        let artist = adw::EntryRow::builder().title("Artist").build();
        let album = adw::EntryRow::builder().title("Album").build();

        let search = adw::ButtonRow::builder().title("Search").build();
        search.add_css_class("suggested-action");

        let query_group = adw::PreferencesGroup::builder().build();
        query_group.add(&artist);
        query_group.add(&album);
        query_group.add(&search);

        let matches_list = list_box();
        let matches_group = adw::PreferencesGroup::builder()
            .title("Matches")
            .visible(false)
            .build();
        matches_group.add(&matches_list);

        let misses_list = list_box();
        let misses_group = adw::PreferencesGroup::builder()
            .title("Also close")
            .description("Records with similar names, kept apart from the matches above")
            .visible(false)
            .build();
        misses_group.add(&misses_list);

        let results = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .build();
        results.append(&matches_group);
        results.append(&misses_group);

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&results)
            .build();

        let empty = adw::StatusPage::builder()
            .icon_name("system-search-symbolic")
            .title("Find an Album")
            .description("Search by artist and title. Sleeve will tell you where to buy it.")
            .build();

        let nothing = adw::StatusPage::builder()
            .icon_name("edit-find-symbolic")
            .title("No Matches")
            .description("Try fewer words, or leave the artist blank.")
            .build();

        let spinner = adw::Spinner::builder()
            .width_request(32)
            .height_request(32)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();

        let stack = gtk::Stack::builder()
            .vexpand(true)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&nothing, Some("nothing"));
        stack.add_named(&spinner, Some("searching"));
        stack.add_named(&scroller, Some("results"));
        stack.set_visible_child_name("empty");

        let banner = adw::Banner::builder().revealed(false).build();

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .margin_top(18)
            .margin_bottom(18)
            .margin_start(18)
            .margin_end(18)
            .build();
        content.append(&query_group);
        content.append(&stack);

        let clamp = adw::Clamp::builder()
            .maximum_size(720)
            .child(&content)
            .build();

        let outer = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        outer.append(&banner);
        outer.append(&clamp);

        let header = adw::HeaderBar::new();
        let toolbar = adw::ToolbarView::builder().content(&outer).build();
        toolbar.add_top_bar(&header);

        let page = adw::NavigationPage::builder()
            .title("Sleeve")
            .tag("search")
            .child(&toolbar)
            .build();

        let this = Rc::new(SearchPage {
            page,
            artist: artist.clone(),
            album: album.clone(),
            stack,
            matches_group,
            misses_group,
            matches_list,
            misses_list,
            banner,
            on_search: Rc::new(RefCell::new(None)),
            on_choose: Rc::new(RefCell::new(None)),
        });

        // Enter in either field searches, which is what anyone will try first.
        for entry in [&artist, &album] {
            let this = Rc::clone(&this);
            entry.connect_entry_activated(move |_| this.fire_search());
        }
        {
            let this = Rc::clone(&this);
            search.connect_activated(move |_| this.fire_search());
        }

        this
    }

    fn fire_search(&self) {
        let query = Query::new(&self.artist.text(), &self.album.text());
        if query.is_empty() {
            return;
        }
        if let Some(handler) = self.on_search.borrow().as_ref() {
            handler(query);
        }
    }

    pub fn connect_search<F: Fn(Query) + 'static>(&self, handler: F) {
        *self.on_search.borrow_mut() = Some(Box::new(handler));
    }

    pub fn connect_choose<F: Fn(Mbid) + 'static>(&self, handler: F) {
        *self.on_choose.borrow_mut() = Some(Box::new(handler));
    }

    pub fn set_searching(&self, searching: bool) {
        if searching {
            self.stack.set_visible_child_name("searching");
        }
    }

    /// A condition the person needs to know about and can fix.
    ///
    /// A banner rather than a toast: a missing Discogs token or an unreadable
    /// config is an ongoing state, and a toast is missed while typing.
    pub fn set_banner(&self, message: Option<&str>) {
        match message {
            Some(text) => {
                self.banner.set_title(text);
                self.banner.set_revealed(true);
            }
            None => self.banner.set_revealed(false),
        }
    }

    pub fn set_matches(&self, matches: &Matches, covers: Option<&Covers>) {
        clear(&self.matches_list);
        clear(&self.misses_list);

        if matches.is_empty() {
            self.stack.set_visible_child_name("nothing");
            return;
        }

        self.matches_group
            .set_visible(!matches.candidates.is_empty());
        self.misses_group
            .set_visible(!matches.near_misses.is_empty());

        // Say so when the top answer is not clearly the answer, rather than
        // letting a list of fifteen near-identical editions imply that the first
        // one is right.
        self.matches_group.set_description(
            match (matches.confidence(), matches.candidates.len()) {
                (Confidence::Confident, _) => None,
                (Confidence::Ambiguous, 0) => None,
                (Confidence::Ambiguous, _) => {
                    Some("Several of these fit equally well — pick the one you meant")
                }
            },
        );

        for candidate in &matches.candidates {
            let (row, picture) = candidate_row::candidate(candidate);
            self.hang_cover(candidate, &picture, covers);
            self.wire(&row, candidate.group.mbid.clone());
            self.matches_list.append(&row);
        }

        for miss in &matches.near_misses {
            let (row, picture) = candidate_row::near_miss(miss);
            self.hang_cover(&miss.candidate, &picture, covers);
            self.wire(&row, miss.candidate.group.mbid.clone());
            self.misses_list.append(&row);
        }

        self.stack.set_visible_child_name("results");
    }

    fn hang_cover(
        &self,
        candidate: &crate::model::candidate::Candidate,
        picture: &gtk::Picture,
        covers: Option<&Covers>,
    ) {
        let Some(covers) = covers else { return };
        let group = &candidate.group;
        let chain = chain(
            None,
            Some(&group.mbid),
            None,
            None,
            &group.artist.name,
            &group.title,
            Size::Thumbnail,
        );
        bind(covers, group.mbid.as_str(), chain, picture);
    }

    fn wire(&self, row: &adw::ActionRow, mbid: Mbid) {
        let handler = Rc::clone(&self.on_choose);
        row.connect_activated(move |_| {
            if let Some(handler) = handler.borrow().as_ref() {
                handler(mbid.clone());
            }
        });
    }
}

fn list_box() -> gtk::ListBox {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list.add_css_class("boxed-list");
    list
}

fn clear(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}
