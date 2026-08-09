//! The drill-down: which pressing of this album.
//!
//! A release group holds the original, the remaster, the deluxe edition, the
//! regional variants and the vinyl reissue, and they are not the same product.
//! This page is where that distinction stops being an abstraction: each row is
//! one issue, with the date, country, format, track count and label that tell it
//! from its neighbours.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use super::art::{bind, Covers};
use super::candidate_row;
use crate::model::album::{Release, ReleaseGroup};
use crate::model::source::art::{chain, Size};

type Handler<T> = Rc<RefCell<Option<Box<dyn Fn(T)>>>>;

pub struct EditionsPage {
    pub page: adw::NavigationPage,
    stack: gtk::Stack,
    group_box: gtk::Box,
    on_choose: Handler<Release>,
}

impl EditionsPage {
    pub fn new() -> Rc<EditionsPage> {
        let group_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .build();

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(
                &adw::Clamp::builder()
                    .maximum_size(720)
                    .child(&group_box)
                    .build(),
            )
            .build();

        let spinner = adw::Spinner::builder()
            .width_request(32)
            .height_request(32)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();

        let nothing = adw::StatusPage::builder()
            .icon_name("dialog-warning-symbolic")
            .title("No Releases Listed")
            .description("MusicBrainz has this album but no specific issues of it.")
            .build();

        let stack = gtk::Stack::builder().vexpand(true).build();
        stack.add_named(&spinner, Some("loading"));
        stack.add_named(&nothing, Some("nothing"));
        stack.add_named(&scroller, Some("releases"));
        stack.set_visible_child_name("loading");

        let toolbar = adw::ToolbarView::builder().content(&stack).build();
        toolbar.add_top_bar(&adw::HeaderBar::new());

        let page = adw::NavigationPage::builder()
            .title("Editions")
            .tag("editions")
            .child(&toolbar)
            .build();

        Rc::new(EditionsPage {
            page,
            stack,
            group_box,
            on_choose: Rc::new(RefCell::new(None)),
        })
    }

    pub fn connect_choose<F: Fn(Release) + 'static>(&self, handler: F) {
        *self.on_choose.borrow_mut() = Some(Box::new(handler));
    }

    pub fn set_loading(&self, group: &ReleaseGroup) {
        self.page.set_title(&group.title);
        self.stack.set_visible_child_name("loading");
    }

    pub fn set_releases(
        &self,
        group: &ReleaseGroup,
        releases: &[Release],
        covers: Option<&Covers>,
    ) {
        while let Some(child) = self.group_box.first_child() {
            self.group_box.remove(&child);
        }
        self.page.set_title(&group.title);

        if releases.is_empty() {
            self.stack.set_visible_child_name("nothing");
            return;
        }

        // Grouped by whether an editor named the edition. Someone hunting a
        // particular remaster wants the named ones together; someone who just
        // wants the album wants the plain issues, and mixing them makes both
        // searches harder.
        let (named, plain): (Vec<&Release>, Vec<&Release>) = releases
            .iter()
            .partition(|release| release.edition().is_some());

        for (title, description, set) in [
            (
                "Editions",
                Some("Remasters, deluxe issues and reissues — these are different masters"),
                named,
            ),
            ("Standard issues", None, plain),
        ] {
            if set.is_empty() {
                continue;
            }
            let list = gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .build();
            list.add_css_class("boxed-list");

            for release in set {
                let (row, picture) = candidate_row::release(release);
                if let Some(covers) = covers {
                    let chain = chain(
                        Some(&release.mbid),
                        Some(&group.mbid),
                        None,
                        None,
                        &release.artist.name,
                        &release.title,
                        Size::Thumbnail,
                    );
                    bind(covers, release.mbid.as_str(), chain, &picture);
                }

                let handler = Rc::clone(&self.on_choose);
                let release = release.clone();
                row.connect_activated(move |_| {
                    if let Some(handler) = handler.borrow().as_ref() {
                        handler(release.clone());
                    }
                });
                list.append(&row);
            }

            let group_widget = adw::PreferencesGroup::builder().title(title).build();
            if let Some(description) = description {
                group_widget.set_description(Some(description));
            }
            group_widget.add(&list);
            self.group_box.append(&group_widget);
        }

        self.stack.set_visible_child_name("releases");
    }
}
