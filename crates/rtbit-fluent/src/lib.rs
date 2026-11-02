//! rustTorrent native client on the fluent-gpui (Fluent 2) design system.
//!
//! Opens the `FluentApp` window: a `Workspace` with the nav sidebar, screen
//! routing and the status bar. Screen bodies are placeholders until the
//! torrent grid (WI-2762) and the async bridge (WI-2760) land.

use fluent_app::{FluentApp, WindowTitle};
use fluent_layout::Workspace;
use gpui::{App, AppContext, Bounds, px, size};
use rtbit_ui_core::Connection;

use window_state::{Rect, WindowState, clamp_to_displays};

pub mod bridge;
pub mod screens;
pub mod shell;
pub mod table_prefs;
pub mod widgets;
pub mod window_state;

/// Window title shown in the title bar.
pub const WINDOW_TITLE: &str = "rustTorrent";
/// Initial window size in logical pixels.
pub const WINDOW_SIZE: (f32, f32) = (1100.0, 720.0);

/// Open the rustTorrent window: frameless `FluentApp` with the fluent title bar.
pub fn run() {
    if std::env::args().any(|arg| arg == "--version") {
        println!("rtbit-fluent {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let state_path = WindowState::default_path();
    let saved = state_path
        .as_deref()
        .map(WindowState::load)
        .unwrap_or_default();
    let restore_screen = saved.screen;
    let (w, h) = saved
        .bounds
        .filter(|b| b.w >= window_state::MIN_EDGE && b.h >= window_state::MIN_EDGE)
        .map_or(WINDOW_SIZE, |b| (b.w, b.h));
    let close_path = state_path.clone();
    FluentApp::new(WINDOW_TITLE)
        .window_size(w, h)
        .on_window_should_close(move |window, _cx| {
            if let Some(path) = &close_path {
                let mut state = WindowState::load(path);
                let b = window.bounds();
                state.maximized = window.is_maximized();
                // A maximized window's bounds are the screen, not the size to come back to.
                if !state.maximized {
                    state.bounds = Some(rect_of(b));
                }
                let _ = state.save(path);
            }
            true
        })
        .run(move |cx| {
            fit_window_to_displays(&saved, cx);
            screens::main_screen::bind_keys(cx);
            let status = cx.new(|_| widgets::status_bar::StatusBar::new());
            let main = cx.new(|cx| screens::MainScreen::new(status.clone(), cx));
            let shell = cx.new(|cx| shell::Shell::new(status.clone(), main, cx));
            WindowTitle::set_status(cx, Connection::Disconnected.label());
            shell.update(cx, |shell, cx| {
                shell.remember_screen(state_path, restore_screen);
                shell.try_resume(cx);
            });
            cx.new(|cx| {
                Workspace::new(cx)
                    .content(shell.into())
                    .status_bar(status.into())
            })
        });
}

fn rect_of(b: Bounds<gpui::Pixels>) -> Rect {
    Rect::new(
        f32::from(b.origin.x),
        f32::from(b.origin.y),
        f32::from(b.size.width),
        f32::from(b.size.height),
    )
}

/// Once the window exists, shrink it to the display it was saved on, drop saved bounds that
/// are off every current display, and re-maximize. The platform picks the position.
fn fit_window_to_displays(saved: &WindowState, cx: &mut App) {
    let saved = saved.clone();
    cx.defer(move |cx| {
        let displays: Vec<Rect> = cx.displays().iter().map(|d| rect_of(d.bounds())).collect();
        let fitted = saved.bounds.and_then(|b| clamp_to_displays(b, &displays));
        let Some(handle) = cx.windows().into_iter().next() else {
            return;
        };
        let _ = handle.update(cx, |_, window, _| {
            match fitted {
                Some(b) => window.resize(size(px(b.w), px(b.h))),
                None => window.resize(size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1))),
            }
            if saved.maximized && !window.is_maximized() {
                window.zoom_window();
            }
        });
    });
}
