//! The half that links no GTK and opens no socket.
//!
//! Everything below is a pure function over data, with two deliberate
//! exceptions: [`cache`] talks to a local SQLite file and [`settings`] reads one
//! file at startup. Neither touches the network, both are deterministic, and
//! both are tested against real storage rather than a fake — the seam worth
//! defending here is the network one, not the disk.
//!
//! Read in this order: [`album`] for identity, [`query`] and [`search`] for
//! finding it, [`offer`] and [`tier`] for what a shop will do, [`score`] for the
//! ranking, and [`verdict`] for the answer.

pub mod album;
pub mod cache;
pub mod candidate;
pub mod offer;
pub mod query;
pub mod score;
pub mod search;
pub mod settings;
pub mod source;
pub mod tier;
pub mod verdict;
pub mod weights;
