//! A row in the ranking: what it is, what it costs, and why it placed there.
//!
//! Every row can show its working. The score on its own is a number someone has
//! to trust; expanded, it is the arithmetic that produced it, line by line, and
//! trust stops being necessary.

use adw::prelude::*;
use gtk::glib;

use crate::model::offer::Acquisition;
use crate::model::score::ScoredOffer;
use crate::model::source::SourceId;

/// One offer.
///
/// `rank` is `None` for the offers outside the ranking — the region-locked ones.
/// Numbering those would imply they are placed against the others when the whole
/// point is that they are not.
pub fn offer(rank: Option<usize>, scored: &ScoredOffer) -> adw::ExpanderRow {
    let title = match rank {
        Some(rank) => format!("{rank}. {}", scored.offer.vendor),
        None => scored.offer.vendor.to_string(),
    };
    let mut subtitle = vec![scored.offer.delivery.describe()];
    match &scored.offer.price {
        Some(price) => subtitle.push(price.to_string()),
        // Say so in the row rather than only inside the expander. A row showing
        // a format and no price otherwise reads as free, or as a bug.
        None if scored.offer.acquisition == Acquisition::Purchase => {
            subtitle.push("price on the shop's page".to_string())
        }
        None => {}
    }
    if let Some(edition) = &scored.offer.edition {
        subtitle.push(edition.clone());
    }

    let row = adw::ExpanderRow::builder()
        .title(glib::markup_escape_text(&title))
        .subtitle(glib::markup_escape_text(&subtitle.join(" · ")))
        .build();

    row.add_prefix(&tier_badge(scored));

    let score = gtk::Label::builder()
        .label(scored.score.to_string())
        .valign(gtk::Align::Center)
        .build();
    score.add_css_class("numeric");
    score.add_css_class("title-4");
    score.set_tooltip_text(Some("Score — higher is better"));
    row.add_suffix(&score);

    if let Some(url) = &scored.offer.url {
        let open = gtk::Button::builder()
            .icon_name("adw-external-link-symbolic")
            .tooltip_text("Open in Browser")
            .valign(gtk::Align::Center)
            .build();
        open.add_css_class("flat");

        // `UriLauncher` rather than `LinkButton`, which paints itself as a blue
        // underlined link and would be the only one on a page of buttons.
        let url = url.clone();
        open.connect_clicked(move |button| {
            let window = button.root().and_downcast::<gtk::Window>();
            gtk::UriLauncher::new(&url).launch(
                window.as_ref(),
                gtk::gio::Cancellable::NONE,
                |_| {},
            );
        });
        row.add_suffix(&open);
    }

    // The payout note first: it is the reason this row is above or below its
    // neighbours far more often than the format is.
    row.add_row(&note_row(
        "Artist payout",
        scored.offer.vendor.payout_note(),
    ));

    for caveat in &scored.caveats {
        row.add_row(&note_row("Note", caveat));
    }

    row.add_row(&breakdown(scored));
    row
}

/// A coloured letter for the tier.
fn tier_badge(scored: &ScoredOffer) -> gtk::Widget {
    let label = gtk::Label::builder()
        .label(scored.tier.letter())
        .width_request(28)
        .height_request(28)
        .valign(gtk::Align::Center)
        .build();
    label.add_css_class("tier-badge");
    label.add_css_class(&format!("tier-{}", scored.tier.letter().to_lowercase()));
    label.set_tooltip_text(Some(scored.tier.description()));
    label.upcast()
}

fn note_row(title: &str, body: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(title))
        .subtitle(glib::markup_escape_text(body))
        .build();
    row.add_css_class("property");
    row
}

/// The arithmetic, one line per component.
fn breakdown(scored: &ScoredOffer) -> gtk::Widget {
    let box_ = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(12)
        .margin_end(12)
        .build();

    for component in &scored.components {
        let line = gtk::Box::builder().spacing(12).build();
        let label = gtk::Label::builder()
            .label(&component.label)
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .build();
        label.add_css_class("caption");

        let delta = gtk::Label::builder()
            .label(format!("{:+}", component.delta))
            .xalign(1.0)
            .build();
        delta.add_css_class("caption");
        delta.add_css_class("numeric");
        delta.add_css_class(if component.delta < 0 {
            "error"
        } else {
            "dimmed"
        });

        line.append(&label);
        line.append(&delta);
        box_.append(&line);
    }

    let total = gtk::Box::builder().spacing(12).margin_top(4).build();
    let label = gtk::Label::builder()
        .label("Total")
        .xalign(0.0)
        .hexpand(true)
        .build();
    label.add_css_class("caption-heading");
    let value = gtk::Label::builder()
        .label(scored.score.to_string())
        .xalign(1.0)
        .build();
    value.add_css_class("caption-heading");
    value.add_css_class("numeric");
    total.append(&label);
    total.append(&value);
    box_.append(&total);

    box_.upcast()
}

/// A row for a source that could not be consulted.
///
/// Shown rather than omitted. A ranking missing Bandcamp is missing its winner
/// most of the time, and an incomplete answer that looks complete is worse than
/// a visibly incomplete one.
pub fn unavailable(source: SourceId, reason: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(source.label()))
        .subtitle(glib::markup_escape_text(reason))
        .build();

    let icon = gtk::Image::from_icon_name("network-offline-symbolic");
    icon.add_css_class("dimmed");
    row.add_prefix(&icon);
    row.add_css_class("dimmed");
    row
}
