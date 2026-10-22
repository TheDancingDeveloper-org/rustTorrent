//! Log view: a live stream of fake log lines with level filter, text filter, pause and clear.

use std::collections::VecDeque;

use egui::{Color32, RichText, ScrollArea, Ui};
use fluent2_egui::controls::{Appearance, Button, ButtonSize};
use fluent2_egui::theme::{Theme, c};
use fluent2_egui::{IconName, SearchBox, ToggleButton, TypeRamp, font_id, rich};

use crate::data::Rng;

const MAX_LINES: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    const ALL: [Level; 5] = [
        Level::Trace,
        Level::Debug,
        Level::Info,
        Level::Warn,
        Level::Error,
    ];

    fn label(self) -> &'static str {
        match self {
            Level::Trace => "TRACE",
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

pub struct LogLine {
    pub time: String,
    pub level: Level,
    pub target: &'static str,
    pub message: String,
}

pub struct LogState {
    pub lines: VecDeque<LogLine>,
    pub paused: bool,
    pub filter: String,
    /// Which of [`Level::ALL`] are shown.
    pub levels: [bool; 5],
    pub follow: bool,
    rng: Rng,
    seconds: u64,
    /// Lines that arrived while paused.
    pub missed: usize,
}

impl Default for LogState {
    fn default() -> Self {
        let mut state = Self {
            lines: VecDeque::new(),
            paused: false,
            filter: String::new(),
            levels: [true; 5],
            follow: true,
            rng: Rng::new(0x10c),
            seconds: 4 * 3600 + 12 * 60,
            missed: 0,
        };
        for _ in 0..160 {
            state.push_random();
        }
        state
    }
}

impl LogState {
    /// Called once per animation tick: append a couple of lines unless paused.
    pub fn tick(&mut self, names: &[&str]) {
        let n = 1 + self.rng.below(3);
        for _ in 0..n {
            if self.paused {
                self.missed += 1;
                self.seconds += 1;
            } else {
                self.push_named(names);
            }
        }
    }

    fn push_random(&mut self) {
        self.push_named(&[
            "ubuntu-24.04.3-desktop-amd64.iso",
            "Sintel.2010.1080p.BluRay.x264",
        ]);
    }

    fn push_named(&mut self, names: &[&str]) {
        self.seconds += self.rng.below(2) as u64;
        let name = if names.is_empty() {
            "torrent"
        } else {
            *self.rng.pick(names)
        };
        let ip = format!(
            "{}.{}.{}.{}:{}",
            self.rng.below(223) + 1,
            self.rng.below(255),
            self.rng.below(255),
            self.rng.below(254) + 1,
            1024 + self.rng.below(60_000)
        );
        let piece = self.rng.below(2400);
        let (level, target, message) = match self.rng.below(14) {
            0 => (
                Level::Warn,
                "librtbit::peer_connection",
                format!("{ip}: handshake timed out"),
            ),
            1 => (
                Level::Error,
                "librtbit::tracker_comms",
                format!("{name}: announce failed: connection refused"),
            ),
            2 | 3 => (
                Level::Info,
                "librtbit::torrent_state",
                format!("{name}: piece {piece} verified"),
            ),
            4 => (
                Level::Info,
                "librtbit::session",
                format!("{name}: added, {} peers from DHT", self.rng.below(80)),
            ),
            5 => (
                Level::Trace,
                "librtbit::peer_connection",
                format!("{ip}: sent request piece={piece} offset=0 len=16384"),
            ),
            6 => (
                Level::Warn,
                "librtbit::dht",
                "bootstrap node router.bittorrent.com:6881 did not respond".into(),
            ),
            7 => (
                Level::Info,
                "librtbit::http_api",
                "GET /api/v2/torrents/info 200 1.2ms".into(),
            ),
            8 => (
                Level::Debug,
                "librtbit::upnp",
                "port mapping 4240/TCP renewed for 3600s".into(),
            ),
            9 => (
                Level::Trace,
                "librtbit::chunk_tracker",
                format!("{name}: marked piece {piece} as needed"),
            ),
            _ => (
                Level::Debug,
                "librtbit::peer_connection",
                format!("{ip}: unchoked us"),
            ),
        };
        let s = self.seconds;
        self.lines.push_back(LogLine {
            time: format!(
                "2026-10-22T{:02}:{:02}:{:02}.{:03}Z",
                s / 3600 % 24,
                s / 60 % 60,
                s % 60,
                self.rng.below(1000)
            ),
            level,
            target,
            message,
        });
        while self.lines.len() > MAX_LINES {
            self.lines.pop_front();
        }
    }
}

fn level_color(theme: &Theme, level: Level) -> Color32 {
    match level {
        Level::Trace => c(theme.color.neutral_foreground_4),
        Level::Debug => c(theme.color.neutral_foreground_3),
        Level::Info => c(theme.color.brand_foreground_1),
        Level::Warn => c(theme.color.status_warning_foreground_1),
        Level::Error => c(theme.color.status_danger_foreground_1),
    }
}

pub fn show(ui: &mut Ui, state: &mut LogState) {
    let theme = Theme::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(rich("rustTorrent server logs", TypeRamp::Subtitle1));
        ui.add_space(16.0);
        for (level, on) in Level::ALL.iter().zip(state.levels.iter_mut()) {
            ui.add(
                ToggleButton::new(on, level.label())
                    .size(ButtonSize::Small)
                    .id_salt(("lvl", level.label())),
            );
        }
        ui.add_space(8.0);
        let (icon, label) = if state.paused {
            (IconName::ArrowRight, "Resume")
        } else {
            (IconName::Subtract, "Pause")
        };
        if ui
            .add(Button::new(label).icon(icon).id_salt("log_pause"))
            .clicked()
        {
            state.paused = !state.paused;
            if !state.paused {
                state.missed = 0;
            }
        }
        ui.add(
            ToggleButton::new(&mut state.follow, "Follow")
                .size(ButtonSize::Small)
                .id_salt("follow"),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(
                    Button::new("Clear")
                        .icon(IconName::Delete)
                        .appearance(Appearance::Subtle),
                )
                .clicked()
            {
                state.lines.clear();
            }
            if ui
                .add(
                    Button::new("Copy")
                        .icon(IconName::Copy)
                        .appearance(Appearance::Subtle),
                )
                .clicked()
            {
                let text: Vec<String> = state
                    .lines
                    .iter()
                    .map(|l| format!("{} {} {} {}", l.time, l.level.label(), l.target, l.message))
                    .collect();
                ui.ctx().copy_text(text.join("\n"));
            }
            ui.add(
                SearchBox::new(&mut state.filter)
                    .placeholder("Filter (regex)...")
                    .desired_width(240.0),
            );
        });
    });
    let needle = state.filter.to_lowercase();
    let visible: Vec<&LogLine> = state
        .lines
        .iter()
        .filter(|l| state.levels[l.level as usize])
        .filter(|l| {
            needle.is_empty()
                || l.message.to_lowercase().contains(&needle)
                || l.target.contains(&needle)
        })
        .collect();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "Showing {} of {} lines (last {MAX_LINES} since window opened)",
                visible.len(),
                state.lines.len()
            ))
            .weak(),
        );
        if state.paused {
            ui.add(
                fluent2_egui::Badge::new(format!("Paused · {} new lines held", state.missed))
                    .color(fluent2_egui::BadgeColor::Warning)
                    .size(fluent2_egui::BadgeSize::Small),
            );
        }
    });
    ui.add_space(4.0);
    let mono = egui::FontId::monospace(font_id(TypeRamp::Caption1).size);
    let row_h = ui.fonts_mut(|f| f.row_height(&mono)) + 2.0;
    egui::Frame::new()
        .fill(c(theme.color.neutral_background_2))
        .stroke(egui::Stroke::new(1.0, c(theme.color.neutral_stroke_2)))
        .corner_radius(4.0)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ScrollArea::vertical()
                .auto_shrink(false)
                .stick_to_bottom(state.follow)
                .show_rows(ui, row_h, visible.len(), |ui, range| {
                    for line in &visible[range] {
                        // One galley per line keeps rows exactly `row_h` tall.
                        let mut job = egui::text::LayoutJob::default();
                        let mut part = |text: &str, color: Color32| {
                            job.append(text, 0.0, egui::TextFormat::simple(mono.clone(), color));
                        };
                        let muted = c(theme.color.neutral_foreground_3);
                        let body = if line.level == Level::Error {
                            level_color(&theme, Level::Error)
                        } else {
                            c(theme.color.neutral_foreground_1)
                        };
                        part(&line.time, muted);
                        part("  ", muted);
                        part(
                            &format!("{:5}", line.level.label()),
                            level_color(&theme, line.level),
                        );
                        part("  ", muted);
                        part(line.target, muted);
                        part("  ", muted);
                        part(&line.message, body);
                        ui.add(egui::Label::new(job).truncate());
                    }
                });
        });
}
