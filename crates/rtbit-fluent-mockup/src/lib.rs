//! Static Fluent 2 mockup of the rustTorrent UI, built on fluent2-egui with fake data and no
//! backend. It mirrors the screens of the React web UI (`crates/librtbit/webui`) so the Fluent
//! look can be judged before any real port.

mod app;
mod data;
mod detail;
mod log;
mod login;
mod rss;
mod settings;
mod table;
mod widgets;

pub use app::{Launch, MockupApp, Screen, StartDialog};
