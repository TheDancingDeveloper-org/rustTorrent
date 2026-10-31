//! rustTorrent native client on the fluent-gpui (Fluent 2) design system.
//!
//! Scaffold only: opens an empty `FluentApp` window. The torrent UI lands in
//! later slices of WI-2758.

use fluent_app::FluentApp;
use gpui::{div, prelude::*, Context, IntoElement, Render, Window};

/// Window title shown in the title bar.
pub const WINDOW_TITLE: &str = "rustTorrent";
/// Initial window size in logical pixels.
pub const WINDOW_SIZE: (f32, f32) = (1100.0, 720.0);

/// Empty root view; the content area is intentionally blank for now.
pub struct FluentRoot;

impl Render for FluentRoot {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

/// Open the rustTorrent window: frameless `FluentApp` with the fluent title bar.
pub fn run() {
    let (w, h) = WINDOW_SIZE;
    FluentApp::new(WINDOW_TITLE)
        .window_size(w, h)
        .run(|cx| cx.new(|_| FluentRoot));
}

