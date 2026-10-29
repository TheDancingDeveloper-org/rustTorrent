//! Static fluent2 mockup of the rustTorrent UI.
//!
//! Fake torrents only. Nothing here talks to a daemon, a disk or the network.
//! It is a workspace member but not a default member, so `cargo build`
//! and the release path (`cargo build -p rtbit`) never compile it.

mod data;
mod ui;

pub use ui::{Mockup, install_theme};
