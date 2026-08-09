//! Rows for the two kinds of search result.
//!
//! Both carry a sleeve, because different editions have different covers and
//! recognising one at a glance is most of what makes a candidate list usable.

use adw::prelude::*;

use crate::model::candidate::{Candidate, NearMiss, NearMissKind};

/// The sleeve size in a search result row.
pub const SLEEVE: i32 = 56;

/// A row for a confident match.
///
/// Returns the row and its picture, so the caller can hang a cover on it when
/// one arrives without holding the whole row.
pub fn candidate(candidate: &Candidate) -> (adw::ActionRow, gtk::Picture) {
    let (frame, picture) = super::art::sleeve(SLEEVE);

    let subtitle = {
        let mut text = candidate.group.subtitle();
        if candidate.group.release_count > 1 {
            if !text.is_empty() {
                text.push_str(" · ");
            }
            text.push_str(&format!("{} releases", candidate.group.release_count));
        }
        text
    };

    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&candidate.group.title))
        .subtitle(glib::markup_escape_text(&subtitle))
        .activatable(true)
        .build();
    row.add_prefix(&frame);

    // The artist goes in a suffix label rather than in the subtitle: on a
    // soundtrack or a classical record the performer is the thing that tells two
    // otherwise identical rows apart, and burying it in a middle dot list is how
    // it gets missed.
    let artist = gtk::Label::builder()
        .label(&candidate.group.artist.name)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(24)
        .xalign(1.0)
        .build();
    artist.add_css_class("dimmed");
    row.add_suffix(&artist);
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

    (row, picture)
}

/// A row for a near miss, labelled with why it is being shown.
pub fn near_miss(miss: &NearMiss) -> (adw::ActionRow, gtk::Picture) {
    let (row, picture) = candidate(&miss.candidate);
    row.set_subtitle(&glib::markup_escape_text(&format!(
        "{} · {}",
        match miss.kind {
            NearMissKind::SameArtistOtherRelease => "Also by this artist",
            NearMissKind::OtherArtistSimilarTitle => "Different artist, similar title",
        },
        miss.candidate.group.subtitle()
    )));
    (row, picture)
}

/// A row for one specific release in the drill-down.
pub fn release(release: &crate::model::album::Release) -> (adw::ActionRow, gtk::Picture) {
    let (frame, picture) = super::art::sleeve(SLEEVE);

    let title = match release.edition() {
        Some(edition) => format!("{} — {}", release.title, edition),
        None => release.title.clone(),
    };

    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&title))
        .subtitle(glib::markup_escape_text(&release.subtitle()))
        .activatable(true)
        .build();
    row.add_prefix(&frame);
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    (row, picture)
}

use gtk::glib;
