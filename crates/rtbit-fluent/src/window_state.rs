//! Window geometry and the last screen, kept in a small local JSON file so the
//! next launch opens where the last one closed.

use std::path::{Path, PathBuf};

use rtbit_ui_core::Screen;
use serde_json::{Value, json};

const FILE_NAME: &str = "window.json";

/// Smallest window edge a stored size may have, so a bad file cannot open a sliver.
pub const MIN_EDGE: f32 = 320.0;
/// How much of the window must overlap a display (width, height) to count as on it.
const MIN_VISIBLE: (f32, f32) = (100.0, 40.0);

/// A rectangle in logical pixels, in the desktop's shared coordinate space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    #[must_use]
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    fn is_finite(&self) -> bool {
        [self.x, self.y, self.w, self.h]
            .iter()
            .all(|v| v.is_finite())
    }

    /// Overlapping width and height with `other`, zero when they do not meet.
    fn overlap(&self, other: &Rect) -> (f32, f32) {
        let w = (self.x + self.w).min(other.x + other.w) - self.x.max(other.x);
        let h = (self.y + self.h).min(other.y + other.h) - self.y.max(other.y);
        (w.max(0.0), h.max(0.0))
    }
}

/// Fit saved bounds to the current displays. Bounds that no longer touch any display
/// (a monitor was unplugged) give `None`; otherwise the window is shrunk to the display
/// it mostly sits on and moved fully onto it.
#[must_use]
pub fn clamp_to_displays(saved: Rect, displays: &[Rect]) -> Option<Rect> {
    if !saved.is_finite() || saved.w < MIN_EDGE || saved.h < MIN_EDGE {
        return None;
    }
    let (display, (ow, oh)) = displays
        .iter()
        .map(|d| (d, saved.overlap(d)))
        .max_by(|a, b| (a.1.0 * a.1.1).total_cmp(&(b.1.0 * b.1.1)))?;
    if ow < MIN_VISIBLE.0 || oh < MIN_VISIBLE.1 {
        return None;
    }
    let w = saved.w.min(display.w);
    let h = saved.h.min(display.h);
    let x = saved.x.clamp(display.x, display.x + display.w - w);
    let y = saved.y.clamp(display.y, display.y + display.h - h);
    Some(Rect::new(x, y, w, h))
}

/// What is remembered between launches.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WindowState {
    pub bounds: Option<Rect>,
    pub maximized: bool,
    pub screen: Option<Screen>,
}

fn screen_name(screen: Screen) -> Option<&'static str> {
    match screen {
        Screen::Main => Some("main"),
        Screen::Rss => Some("rss"),
        Screen::Settings => Some("settings"),
        // Sign-in screens are never worth returning to.
        _ => None,
    }
}

fn screen_from_name(name: &str) -> Option<Screen> {
    match name {
        "main" => Some(Screen::Main),
        "rss" => Some(Screen::Rss),
        "settings" => Some(Screen::Settings),
        _ => None,
    }
}

impl WindowState {
    /// `<config dir>/rtbit-fluent/window.json`, next to the table preferences.
    #[must_use]
    pub fn default_path() -> Option<PathBuf> {
        crate::table_prefs::TablePrefs::default_path().map(|p| p.with_file_name(FILE_NAME))
    }

    /// Read the file; a missing or unreadable file gives the defaults.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .map(|value| Self::from_json(&value))
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(&self.to_json())?)
    }

    fn to_json(&self) -> Value {
        let bounds = self
            .bounds
            .map(|b| json!({ "x": b.x, "y": b.y, "w": b.w, "h": b.h }));
        json!({ "bounds": bounds, "maximized": self.maximized,
            "screen": self.screen.and_then(screen_name) })
    }

    fn from_json(value: &Value) -> Self {
        let bounds = value.get("bounds").and_then(|b| {
            let n = |key| b.get(key)?.as_f64().map(|v| v as f32);
            Some(Rect::new(n("x")?, n("y")?, n("w")?, n("h")?))
        });
        Self {
            bounds: bounds.filter(Rect::is_finite),
            maximized: value
                .get("maximized")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            screen: value
                .get("screen")
                .and_then(Value::as_str)
                .and_then(screen_from_name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRIMARY: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 1920.0,
        h: 1080.0,
    };
    const SECOND: Rect = Rect {
        x: 1920.0,
        y: 0.0,
        w: 1280.0,
        h: 1024.0,
    };

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rtbit-fluent-ws-{name}-{}", std::process::id()))
    }

    #[test]
    fn bounds_on_a_display_are_kept() {
        let saved = Rect::new(100.0, 80.0, 1100.0, 720.0);
        assert_eq!(clamp_to_displays(saved, &[PRIMARY]), Some(saved));
    }

    #[test]
    fn bounds_on_an_unplugged_display_are_discarded() {
        let saved = Rect::new(2100.0, 80.0, 1100.0, 720.0);
        assert_eq!(clamp_to_displays(saved, &[SECOND]), Some(saved));
        assert_eq!(clamp_to_displays(saved, &[PRIMARY]), None);
        assert_eq!(clamp_to_displays(saved, &[]), None);
    }

    #[test]
    fn barely_visible_bounds_are_discarded() {
        let saved = Rect::new(1900.0, 1060.0, 1100.0, 720.0);
        assert_eq!(clamp_to_displays(saved, &[PRIMARY]), None);
    }

    #[test]
    fn partly_off_screen_bounds_move_back_on() {
        let saved = Rect::new(1200.0, -50.0, 1100.0, 720.0);
        let fixed = clamp_to_displays(saved, &[PRIMARY]).unwrap();
        assert_eq!(fixed, Rect::new(820.0, 0.0, 1100.0, 720.0));
    }

    #[test]
    fn oversized_bounds_shrink_to_the_display() {
        let saved = Rect::new(0.0, 0.0, 5000.0, 4000.0);
        let fixed = clamp_to_displays(saved, &[PRIMARY]).unwrap();
        assert_eq!(fixed, PRIMARY);
    }

    #[test]
    fn nonsense_bounds_are_discarded() {
        assert_eq!(
            clamp_to_displays(Rect::new(0.0, 0.0, 10.0, 10.0), &[PRIMARY]),
            None
        );
        assert_eq!(
            clamp_to_displays(Rect::new(f32::NAN, 0.0, 800.0, 600.0), &[PRIMARY]),
            None
        );
    }

    #[test]
    fn round_trips_through_the_file() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("nested").join(FILE_NAME);
        let state = WindowState {
            bounds: Some(Rect::new(10.5, 20.0, 900.0, 700.0)),
            maximized: true,
            screen: Some(Screen::Rss),
        };
        state.save(&path).unwrap();
        assert_eq!(WindowState::load(&path), state);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sign_in_screens_are_not_stored_and_bad_files_give_defaults() {
        let dir = temp_dir("bad");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        assert_eq!(WindowState::load(&path), WindowState::default());
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(WindowState::load(&path), WindowState::default());
        let state = WindowState {
            screen: Some(Screen::Connect),
            ..Default::default()
        };
        state.save(&path).unwrap();
        assert_eq!(WindowState::load(&path).screen, None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
