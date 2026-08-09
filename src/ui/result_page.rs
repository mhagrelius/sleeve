//! The answer: where to buy it, best first, and why.
//!
//! Four things, in this order, because that is the order they matter in: the
//! recommendation in plain words, anything odd about the results, the ranking,
//! and then what could not be checked.
//!
//! That last section is not an apology. With Bandcamp or Qobuz unreachable the
//! ranking is usually missing its winner, and a partial answer that looks whole
//! is worse than one that says what it does not know.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::art::{bind, Covers};
use super::offer_row;
use crate::model::source::art::{chain, Size};
use crate::model::tier::Tier;
use crate::model::verdict::{Conflict, Verdict};

pub struct ResultPage {
    pub page: adw::NavigationPage,
    stack: gtk::Stack,
    content: gtk::Box,
}

impl ResultPage {
    pub fn new() -> Rc<ResultPage> {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .margin_top(18)
            .margin_bottom(18)
            .margin_start(18)
            .margin_end(18)
            .build();

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(
                &adw::Clamp::builder()
                    .maximum_size(720)
                    .child(&content)
                    .build(),
            )
            .build();

        let spinner = adw::Spinner::builder()
            .width_request(32)
            .height_request(32)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();

        let stack = gtk::Stack::builder().vexpand(true).build();
        stack.add_named(&spinner, Some("looking"));
        stack.add_named(&scroller, Some("verdict"));
        stack.set_visible_child_name("looking");

        let toolbar = adw::ToolbarView::builder().content(&stack).build();
        toolbar.add_top_bar(&adw::HeaderBar::new());

        let page = adw::NavigationPage::builder()
            .title("Where to Buy")
            .tag("result")
            .child(&toolbar)
            .build();

        Rc::new(ResultPage {
            page,
            stack,
            content,
        })
    }

    pub fn set_looking(&self, title: &str) {
        self.page.set_title(title);
        self.stack.set_visible_child_name("looking");
    }

    pub fn set_verdict(&self, verdict: &Verdict, covers: Option<&Covers>) {
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }

        if let Some(release) = &verdict.release {
            self.page.set_title(&release.title);
            let (header, picture) = header(release);
            self.content.append(&header);
            if let Some(covers) = covers {
                let chain = chain(
                    Some(&release.mbid),
                    Some(&release.group),
                    None,
                    None,
                    &release.artist.name,
                    &release.title,
                    Size::Full,
                );
                bind(covers, release.mbid.as_str(), chain, &picture);
            }
        }

        // Tier E: a statement about the album, not a row in a list. Nothing here
        // links anywhere, because there is nowhere legitimate to link to.
        if let Some(missing) = &verdict.not_available {
            let status = adw::StatusPage::builder()
                .icon_name("dialog-information-symbolic")
                .title("No Legitimate Purchase Path")
                .description(&missing.reason)
                .build();
            status.add_css_class("compact");
            self.content.append(&status);
        } else {
            self.content.append(&recommendation(verdict));
        }

        // One banner at most. The remaster warning is strictly more specific
        // than "there is more than one master", so when both fire only the
        // sharper one is shown — and the recommendation has already said it
        // once, so a third telling would be noise.
        if let Some(banner) = verdict
            .conflicts
            .iter()
            .find(|conflict| matches!(conflict, Conflict::RemasterIsFlatter { .. }))
            .or_else(|| {
                verdict
                    .conflicts
                    .iter()
                    .find(|conflict| matches!(conflict, Conflict::MultipleMasters { .. }))
            })
            .and_then(conflict_banner)
        {
            self.content.append(&banner);
        }

        if !verdict.ranked.ranked.is_empty() {
            let group = adw::PreferencesGroup::builder().title("Ranked").build();
            let list = boxed_list();
            for (index, scored) in verdict.ranked.ranked.iter().enumerate() {
                list.append(&offer_row::offer(Some(index + 1), scored));
            }
            group.add(&list);
            self.content.append(&group);
        }

        if !verdict.ranked.unavailable_here.is_empty() {
            let group = adw::PreferencesGroup::builder()
                .title("Sold, but not to you")
                .description("These exist, but not in your region — they are not ranked")
                .build();
            let list = boxed_list();
            for scored in &verdict.ranked.unavailable_here {
                list.append(&offer_row::offer(None, scored));
            }
            group.add(&list);
            self.content.append(&group);
        }

        if !verdict.unchecked.is_empty() {
            let group = adw::PreferencesGroup::builder()
                .title("Not checked")
                .description("The ranking above may be missing a better option")
                .build();
            let list = boxed_list();
            for (source, reason) in &verdict.unchecked {
                list.append(&offer_row::unavailable(*source, reason));
            }
            group.add(&list);
            self.content.append(&group);
        }

        self.stack.set_visible_child_name("verdict");
    }
}

/// Sleeve, title, artist and pressing detail.
fn header(release: &crate::model::album::Release) -> (gtk::Widget, gtk::Picture) {
    {
        let (frame, picture) = super::art::sleeve(128);

        // The edition belongs in the title here. "Which master am I buying" is
        // the question the whole page answers, and a header that says only
        // "Blade Runner" for the Esper Edition answers it wrongly.
        let title = gtk::Label::builder()
            .label(match release.edition() {
                Some(edition) => format!("{} — {edition}", release.title),
                None => release.title.clone(),
            })
            .xalign(0.0)
            .wrap(true)
            .build();
        title.add_css_class("title-2");

        let artist = gtk::Label::builder()
            .label(&release.artist.name)
            .xalign(0.0)
            .wrap(true)
            .build();
        artist.add_css_class("dimmed");

        let detail = gtk::Label::builder()
            .label(release.subtitle())
            .xalign(0.0)
            .wrap(true)
            .build();
        detail.add_css_class("caption");
        detail.add_css_class("dimmed");

        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .build();
        text.append(&title);
        text.append(&artist);
        text.append(&detail);

        let row = gtk::Box::builder().spacing(18).build();
        row.append(&frame);
        row.append(&text);
        (row.upcast(), picture)
    }
}

/// The plain-language answer, in a card above the list.
fn recommendation(verdict: &Verdict) -> gtk::Widget {
    let label = gtk::Label::builder()
        .label(&verdict.recommendation)
        .xalign(0.0)
        .wrap(true)
        .build();

    let heading = gtk::Label::builder()
        .label(match verdict.tier() {
            Tier::A => "Buy it",
            Tier::B => "Own it, but lossy",
            Tier::C | Tier::D => "Streaming only",
            Tier::E => "Nowhere to buy it",
        })
        .xalign(0.0)
        .build();
    heading.add_css_class("heading");

    let box_ = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    box_.append(&heading);
    box_.append(&label);
    box_.add_css_class("card");
    // Padding in CSS rather than widget margins: a margin sits outside the
    // border, so the accent stripe ends up against the first character.
    box_.add_css_class("verdict");
    box_.add_css_class(match verdict.tier() {
        Tier::A => "verdict-good",
        Tier::B | Tier::C => "verdict-fair",
        Tier::D | Tier::E => "verdict-poor",
    });
    box_.upcast()
}

/// A banner for something that needs saying before the list is read.
fn conflict_banner(conflict: &Conflict) -> Option<adw::Banner> {
    let title = match conflict {
        // Already the first thing the recommendation says; a banner repeating it
        // would be the same fact twice on one screen.
        Conflict::BestIsNotCheapest { .. } => return None,
        Conflict::RemasterIsFlatter {
            original, remaster, ..
        } => format!(
            "The {} is more compressed than the {} — DR{} against DR{}",
            remaster.edition,
            original.edition,
            remaster.dr.unwrap_or(0),
            original.dr.unwrap_or(0)
        ),
        Conflict::MultipleMasters { editions } => format!(
            "More than one master is being sold: {}",
            editions
                .iter()
                .map(|edition| {
                    let vendors: Vec<&str> = edition.vendors.iter().map(|v| v.label()).collect();
                    format!("{} ({})", edition.edition, vendors.join(", "))
                })
                .collect::<Vec<_>>()
                .join("; ")
        ),
    };

    let banner = adw::Banner::builder()
        .title(glib::markup_escape_text(&title))
        .revealed(true)
        .build();
    Some(banner)
}

fn boxed_list() -> gtk::ListBox {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list.add_css_class("boxed-list");
    list
}
