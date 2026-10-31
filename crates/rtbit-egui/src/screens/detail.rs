//! Detail pane for the selected torrent.
//!
//! Files are a `data::TreeView`: folders are path prefixes and leaves are
//! file indices. Tabs are `surfaces::TabList`. The piece bar is a local
//! stand-in: one coloured segment per piece, capped so a large torrent does
//! not draw tens of thousands of rects.

use std::collections::BTreeMap;

use fluent2_egui::controls::{Appearance, Button, CheckState, Input};
use fluent2_egui::data::tree_view::{Children, TreeNode, TreeState, TreeView};
use fluent2_egui::surfaces::TabList;
use fluent2_egui::{Icon, IconName};
use rtbit_client::{PeerList, PieceProgress, Torrent, TorrentFile, TrackerList};

pub use rtbit_ui_core::DetailAction;

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

/// Draw the pane. Returns a mutation when the user changed the selection or
/// added a tracker. `tracker_url` is the text field the shell keeps.
pub fn draw_detail(
    ui: &mut egui::Ui,
    view: &DetailView,
    tab: &mut DetailTab,
    tracker_url: &mut String,
    tree: &mut TreeState<i64>,
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
            if let Some(next) = files(ui, torrent, tree, busy) {
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

fn files(
    ui: &mut egui::Ui,
    torrent: &Torrent,
    tree: &mut TreeState<i64>,
    busy: bool,
) -> Option<DetailAction> {
    let Some(files) = &torrent.files else {
        ui.label("No file list.");
        return None;
    };
    if files.is_empty() {
        ui.label("No files.");
        return None;
    }
    let index = FileIndex::build(files);
    let mut action = None;
    ui.add_enabled_ui(!busy, |ui| {
        let response = TreeView::new("rtbit_files")
            .label("Files")
            .show(ui, tree, |parent, out| {
                let children = match parent {
                    None => index.roots.as_slice(),
                    Some(key) => index.children.get(key).map(Vec::as_slice).unwrap_or(&[]),
                };
                for node in children {
                    out.push(node_of(node, files, &index));
                }
            });
        if let Some(key) = response.check_toggled {
            action = toggle_action(key, files, &index);
        }
    });
    action
}

/// A folder (`key` negative) or a file (`key` is the file index).
struct Node {
    key: i64,
    label: String,
    folder: bool,
}

struct FileIndex {
    roots: Vec<Node>,
    children: BTreeMap<i64, Vec<Node>>,
    /// File indices under each folder, including nested ones.
    members: BTreeMap<i64, Vec<usize>>,
}

impl FileIndex {
    fn build(files: &[TorrentFile]) -> Self {
        let mut folders: BTreeMap<Vec<String>, i64> = BTreeMap::new();
        let mut children: BTreeMap<i64, Vec<Node>> = BTreeMap::new();
        let mut members: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
        let mut roots: Vec<Node> = Vec::new();
        let mut next_folder: i64 = -1;
        for (index, file) in files.iter().enumerate() {
            let parts = parts_of(file);
            let (dirs, name) = match parts.split_last() {
                Some((name, dirs)) => (dirs, name.clone()),
                None => (&[][..], file.name.clone()),
            };
            let mut parent: Option<i64> = None;
            let mut prefix: Vec<String> = Vec::new();
            for dir in dirs {
                prefix.push(dir.clone());
                let key = if let Some(existing) = folders.get(&prefix) {
                    *existing
                } else {
                    let key = next_folder;
                    next_folder -= 1;
                    folders.insert(prefix.clone(), key);
                    let node = Node {
                        key,
                        label: dir.clone(),
                        folder: true,
                    };
                    match parent {
                        Some(parent_key) => children.entry(parent_key).or_default().push(node),
                        None => roots.push(node),
                    }
                    key
                };
                members.entry(key).or_default().push(index);
                parent = Some(key);
            }
            let node = Node {
                key: i64::try_from(index).unwrap_or(i64::MAX),
                label: name,
                folder: false,
            };
            match parent {
                Some(parent_key) => children.entry(parent_key).or_default().push(node),
                None => roots.push(node),
            }
        }
        Self {
            roots,
            children,
            members,
        }
    }
}

fn parts_of(file: &TorrentFile) -> Vec<String> {
    if file.components.is_empty() {
        if file.name.is_empty() {
            Vec::new()
        } else {
            vec![file.name.clone()]
        }
    } else {
        file.components.clone()
    }
}

/// A folder checkbox sets every file under it. Mixed or unchecked becomes
/// included; a fully checked folder becomes excluded. A file toggles itself.
fn toggle_action(key: i64, files: &[TorrentFile], index: &FileIndex) -> Option<DetailAction> {
    if key < 0 {
        let members = index.members.get(&key)?;
        let included = folder_check(members, files) != CheckState::Checked;
        let indices: Vec<usize> = members
            .iter()
            .copied()
            .filter(|file_index| {
                files
                    .get(*file_index)
                    .is_some_and(|file| file.included != included)
            })
            .collect();
        if indices.is_empty() {
            None
        } else {
            Some(DetailAction::SetIncludedMany { indices, included })
        }
    } else {
        let file_index = usize::try_from(key).ok()?;
        let file = files.get(file_index)?;
        Some(DetailAction::SetIncluded {
            index: file_index,
            included: !file.included,
        })
    }
}

fn node_of(node: &Node, files: &[TorrentFile], index: &FileIndex) -> TreeNode<i64> {
    if node.folder {
        let members = index
            .members
            .get(&node.key)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let checked = folder_check(members, files);
        TreeNode::new(node.key, node.label.clone())
            .children(Children::Some)
            .icon(Icon::new(IconName::Folder))
            .checked(checked)
    } else {
        let included = files
            .get(usize::try_from(node.key).unwrap_or(usize::MAX))
            .is_some_and(|file| file.included);
        TreeNode::new(node.key, node.label.clone())
            .icon(Icon::new(IconName::Document))
            .checked(if included {
                CheckState::Checked
            } else {
                CheckState::Unchecked
            })
    }
}

fn folder_check(members: &[usize], files: &[TorrentFile]) -> CheckState {
    let mut on = 0;
    let mut off = 0;
    for index in members {
        match files.get(*index) {
            Some(file) if file.included => on += 1,
            Some(_) => off += 1,
            None => {}
        }
    }
    if on > 0 && off > 0 {
        CheckState::Mixed
    } else if on > 0 {
        CheckState::Checked
    } else {
        CheckState::Unchecked
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use rtbit_client::Torrent;

    fn file(name: &str, components: &[&str], included: bool) -> TorrentFile {
        TorrentFile {
            name: name.into(),
            components: components.iter().map(|part| (*part).to_owned()).collect(),
            length: 1,
            included,
        }
    }

    fn torrent(files: Vec<TorrentFile>) -> Torrent {
        Torrent {
            id: Some(1),
            info_hash: "a".repeat(40),
            name: Some("Example".into()),
            output_folder: "/tmp".into(),
            total_pieces: 1,
            category: None,
            tags: None,
            trackers: None,
            stats: None,
            files: Some(files),
        }
    }

    #[test]
    fn a_folder_toggle_sets_every_file_under_it() {
        let files = vec![
            file("a.mkv", &["Season", "a.mkv"], false),
            file("b.mkv", &["Season", "b.mkv"], true),
            file("root.txt", &["root.txt"], false),
        ];
        let torrent = torrent(files.clone());
        let files = torrent.files.as_deref().unwrap();
        let index = FileIndex::build(files);
        let season = index
            .roots
            .iter()
            .find(|node| node.label == "Season")
            .expect("season folder");
        assert!(season.folder);
        let action = toggle_action(season.key, files, &index).expect("folder action");
        match action {
            DetailAction::SetIncludedMany { indices, included } => {
                assert!(included, "a mixed folder becomes included");
                assert_eq!(indices, vec![0]);
            }
            other => panic!("folder toggle must cover its files, got {other:?}"),
        }
        // Once every file under the folder is included, the same click excludes them all.
        let all_on = vec![
            file("a.mkv", &["Season", "a.mkv"], true),
            file("b.mkv", &["Season", "b.mkv"], true),
            file("root.txt", &["root.txt"], false),
        ];
        let index = FileIndex::build(&all_on);
        let season = index
            .roots
            .iter()
            .find(|node| node.label == "Season")
            .expect("season folder");
        let action = toggle_action(season.key, &all_on, &index).expect("folder action");
        match action {
            DetailAction::SetIncludedMany { indices, included } => {
                assert!(!included);
                assert_eq!(indices, vec![0, 1]);
            }
            other => panic!("expected every nested file, got {other:?}"),
        }
    }
}
