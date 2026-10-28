//! Detail pane for the selected torrent.
//!
//! fluent2-egui has no tree view (design §6), so files are a flat list with
//! the path as the name. Tabs are `surfaces::TabList`. The piece bar is a
//! local stand-in: one coloured segment per piece, capped so a large torrent
//! does not draw tens of thousands of rects.

use fluent2_egui::controls::{Appearance, Button, Checkbox, Input};
use fluent2_egui::surfaces::TabList;
use rtbit_client::{PeerList, PieceProgress, Torrent, TrackerList};

/// How many piece segments the bar will draw. Wider torrents are sampled.
const PIECE_CAP: usize = 400;

/// Which tab the pane shows. The shell owns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailTab {
    Files,
    Trackers,
    Peers,
    Pieces,
}

impl DetailTab {
    const ALL: [Self; 4] = [Self::Files, Self::Trackers, Self::Peers, Self::Pieces];

    fn label(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Trackers => "Trackers",
            Self::Peers => "Peers",
            Self::Pieces => "Pieces",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }

    fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(Self::Files)
    }
}

/// What the pane knows about the selected torrent.
#[derive(Clone, Debug, Default)]
pub struct DetailView {
    pub torrent: Option<Torrent>,
    pub trackers: Option<TrackerList>,
    pub peers: Option<PeerList>,
    pub pieces: Option<PieceProgress>,
    pub error: Option<String>,
}

/// A change the pane asked for. The shell posts it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetailAction {
    /// Toggle one file in or out of the download selection.
    SetIncluded { index: usize, included: bool },
    /// Attach one announce URL.
    AddTracker(String),
}

/// Draw the pane. Returns a mutation when the user changed the selection or
/// added a tracker. `tracker_url` is the text field the shell keeps.
pub fn draw_detail(
    ui: &mut egui::Ui,
    view: &DetailView,
    tab: &mut DetailTab,
    tracker_url: &mut String,
    busy: bool,
) -> Option<DetailAction> {
    let mut action = None;
    ui.heading("Details");
    let Some(torrent) = &view.torrent else {
        ui.label(if view.error.is_some() {
            view.error.as_deref().unwrap_or("No torrent selected.")
        } else {
            "Select a torrent."
        });
        return None;
    };
    ui.label(torrent.name.as_deref().unwrap_or("(unnamed)"));
    if let Some(error) = &view.error {
        ui.colored_label(egui::Color32::from_rgb(196, 43, 28), error);
    }
    let mut index = tab.index();
    let mut tabs = TabList::new("rtbit_detail_tabs", &mut index);
    for one in DetailTab::ALL {
        tabs = tabs.tab(one.label());
    }
    tabs.show(ui);
    *tab = DetailTab::from_index(index);
    ui.separator();
    match *tab {
        DetailTab::Files => {
            if let Some(next) = files(ui, torrent, busy) {
                action = Some(next);
            }
        }
        DetailTab::Trackers => {
            if let Some(next) = trackers(ui, view, tracker_url, busy) {
                action = Some(next);
            }
        }
        DetailTab::Peers => peers(ui, view),
        DetailTab::Pieces => pieces(ui, view),
    }
    action
}

fn files(ui: &mut egui::Ui, torrent: &Torrent, busy: bool) -> Option<DetailAction> {
    let Some(files) = &torrent.files else {
        ui.label("No file list.");
        return None;
    };
    if files.is_empty() {
        ui.label("No files.");
        return None;
    }
    let mut action = None;
    for (index, file) in files.iter().enumerate() {
        ui.horizontal(|ui| {
            let mut on = file.included;
            let before = on;
            ui.add_enabled_ui(!busy, |ui| {
                ui.add(Checkbox::bool(&mut on, ""));
            });
            if on != before && action.is_none() {
                action = Some(DetailAction::SetIncluded {
                    index,
                    included: on,
                });
            }
            let path = if file.components.is_empty() {
                file.name.clone()
            } else {
                file.components.join("/")
            };
            ui.label(path);
        });
    }
    action
}

fn trackers(
    ui: &mut egui::Ui,
    view: &DetailView,
    tracker_url: &mut String,
    busy: bool,
) -> Option<DetailAction> {
    let mut action = None;
    match &view.trackers {
        Some(list) if list.trackers.is_empty() => {
            ui.label("No trackers.");
        }
        Some(list) => {
            for tracker in &list.trackers {
                let peers = match (tracker.seeders, tracker.leechers) {
                    (Some(seeders), Some(leechers)) => {
                        format!(" · {seeders} seeders, {leechers} leechers")
                    }
                    _ => String::new(),
                };
                ui.label(format!(
                    "{} · {}{peers}",
                    tracker.url,
                    tracker.state.label()
                ));
                if let Some(error) = &tracker.last_error {
                    ui.label(error);
                }
            }
        }
        None => {
            ui.label("Loading trackers…");
        }
    };
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label("Add tracker");
        ui.add(Input::new(tracker_url).placeholder("http://tracker.example/announce"));
        let ready = !busy && looks_like_tracker(tracker_url);
        if ui
            .add_enabled(ready, Button::new("Add").appearance(Appearance::Primary))
            .clicked()
        {
            action = Some(DetailAction::AddTracker(tracker_url.trim().to_owned()));
        }
    });
    action
}

fn peers(ui: &mut egui::Ui, view: &DetailView) {
    match &view.peers {
        Some(list) if list.peers.is_empty() => {
            ui.label("No live peers.");
        }
        Some(list) => {
            for peer in &list.peers {
                ui.label(format!(
                    "{} · {} · down {} · up {}",
                    peer.addr,
                    peer.state,
                    crate::format::bytes(peer.fetched_bytes),
                    crate::format::bytes(peer.uploaded_bytes)
                ));
            }
        }
        None => {
            ui.label("Loading peers…");
        }
    };
}

fn pieces(ui: &mut egui::Ui, view: &DetailView) {
    let Some(pieces) = &view.pieces else {
        ui.label("Loading pieces…");
        return;
    };
    if pieces.have.is_empty() {
        ui.label("No pieces.");
        return;
    }
    ui.label(format!(
        "{} of {} pieces",
        pieces.have_count(),
        pieces.have.len()
    ));
    let samples = sample(&pieces.have);
    let (rect, _response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(1.0), 16.0),
        egui::Sense::hover(),
    );
    let painter = ui.painter();
    let width = rect.width() / samples.len() as f32;
    for (index, have) in samples.iter().enumerate() {
        let x = rect.left() + index as f32 * width;
        let cell =
            egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(width, rect.height()));
        let color = if *have {
            egui::Color32::from_rgb(34, 197, 94)
        } else {
            egui::Color32::from_rgb(55, 65, 81)
        };
        painter.rect_filled(cell, 0.0, color);
    }
}

/// A tracker URL the daemon will accept: http, https or udp, with a host.
fn looks_like_tracker(value: &str) -> bool {
    let value = value.trim();
    let Some(rest) = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"))
        .or_else(|| value.strip_prefix("udp://"))
    else {
        return false;
    };
    let host = rest.split(['/', ':', '?', '#']).next().unwrap_or("");
    !host.is_empty() && host != "."
}

fn sample(have: &[bool]) -> Vec<bool> {
    if have.len() <= PIECE_CAP {
        return have.to_vec();
    }
    let mut out = Vec::with_capacity(PIECE_CAP);
    for index in 0..PIECE_CAP {
        let start = index * have.len() / PIECE_CAP;
        let end = ((index + 1) * have.len() / PIECE_CAP).max(start + 1);
        out.push(have[start..end].iter().any(|bit| *bit));
    }
    out
}
