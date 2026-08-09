//! The half that knows a window exists.
//!
//! Widget trees are built in Rust — no `.ui` XML, no Blueprint, no GResource.
//! The structure of a page is then readable in the same file as the behaviour
//! that drives it, which for an application this size is worth more than a
//! designer could give back. The sibling apps are built the same way.
//!
//! Widgets report what a person did and nothing else. [`SleeveApplication`] is
//! the only object here that asks a source anything, and `http` is the only
//! module in the whole tree that opens a socket.

mod application;
mod art;
mod candidate_row;
mod editions_page;
mod http;
mod offer_row;
mod result_page;
mod search_page;
mod window;

pub use application::SleeveApplication;
pub use editions_page::EditionsPage;
pub use result_page::ResultPage;
pub use search_page::SearchPage;
pub use window::SleeveWindow;

/// The application stylesheet, compiled in.
pub const STYLE: &str = include_str!("style.css");

/// Load the stylesheet at application priority, above the theme and below the
/// user's own overrides.
pub fn load_stylesheet(display: &gtk::gdk::Display) {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(STYLE);
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
