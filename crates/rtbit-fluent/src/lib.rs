//! rustTorrent native client on the fluent-gpui (Fluent 2) design system.
//!
//! Opens the `FluentApp` window: a `Workspace` with the nav sidebar, screen
//! routing and the status bar. Screen bodies are placeholders until the
//! torrent grid (WI-2762) and the async bridge (WI-2760) land.

use fluent_app::{FluentApp, WindowTitle};
use fluent_layout::Workspace;
use gpui::AppContext;
use rtbit_ui_core::Connection;

pub mod shell;
pub mod widgets;

/// Window title shown in the title bar.
pub const WINDOW_TITLE: &str = "rustTorrent";
/// Initial window size in logical pixels.
pub const WINDOW_SIZE: (f32, f32) = (1100.0, 720.0);

/// Open the rustTorrent window: frameless `FluentApp` with the fluent title bar.
pub fn run() {
    let (w, h) = WINDOW_SIZE;
    FluentApp::new(WINDOW_TITLE).window_size(w, h).run(|cx| {
        let status = cx.new(|_| widgets::status_bar::StatusBar::new());
        let shell = cx.new(|_| shell::Shell::new(status.clone()));
        WindowTitle::set_status(cx, Connection::Disconnected.label());
        cx.new(|cx| {
            Workspace::new(cx)
                .content(shell.into())
                .status_bar(status.into())
        })
    });
}
