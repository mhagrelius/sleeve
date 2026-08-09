//! Sleeve: where to buy an album, ranked best-first, with the reasoning shown.
//!
//! Not a "where can I stream it" tool. The ranking is built around owning
//! DRM-free files rather than renting access to them, and around what reaches
//! the artist — see [`model::score`] for the four principles it enforces and the
//! order it enforces them in.
//!
//! Two halves. `model/` links no GTK and opens no socket: a source is a pair of
//! pure functions that build a [`model::source::Request`] and parse a response
//! body into an [`model::source::Outcome`], which is why `cargo test` exercises
//! every source, every failure shape and every ranking with no display and no
//! network. `ui/` is the only half that knows a window exists, and
//! `ui::http` is the only file in the tree that performs a request.

pub mod model;
pub mod ui;

pub const APP_ID: &str = "us.hagreli.Sleeve";
