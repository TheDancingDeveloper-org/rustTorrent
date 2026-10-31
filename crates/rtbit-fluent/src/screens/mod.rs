//! Screens the shell routes to. Each owns its own form state and talks to the daemon only through the bridge.

pub mod add_dialog;
pub mod connect;
pub mod detail;
pub mod main_screen;
pub mod rss;
pub mod settings;

pub use main_screen::MainScreen;
