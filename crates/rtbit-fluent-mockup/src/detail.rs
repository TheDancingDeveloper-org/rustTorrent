//! The detail pane under the table: Overview, Files, Peers, Trackers, Speed and Pieces tabs.

use std::cmp::Ordering;

use egui::{Grid, RichText, ScrollArea, Ui, vec2};
use fluent2_egui::controls::Sparkline;
use fluent2_egui::data::data_grid::{Align, Column, ColumnWidth, SortDirection, SortSpec};
use fluent2_egui::data::tree_view::{Children, TreeNode, TreeSize, TreeState, TreeView};
use fluent2_egui::theme::{Theme, c};
use fluent2_egui::{
    DataGrid, DataGridSize, DataGridState, Icon, IconName, RowId, SelectionMode, TabList, TypeRamp,
    rich,
};

use crate::data::{self, FileNode, Piece, State, Torrent};
use crate::widgets;

pub const TABS: [&str; 6] = ["Overview", "Files", "Peers", "Trackers", "Speed", "Pieces"];

pub struct DetailState {
    pub tab: usize,
    /// Torrent the file tree state belongs to; reset when the selection moves.
    pub files_for: Option<u64>,
    pub files: TreeState<Vec<usize>>,
    pub peers: DataGridState,
    pub trackers: DataGridState,
}

impl Default for DetailState {
    fn default() -> Self {
        Self {
            tab: 0,
            files_for: None,
            files: TreeState::default(),
            peers: DataGridState {
                sort: Some(SortSpec {
                    column: "down".to_owned(),
                    direction: SortDirection::Descending,
                }),
                ..Default::default()
            },
            trackers: DataGridState::default(),
        }
    }
}

pub fn show(ui: &mut Ui, state: &mut DetailState, selected: &[&Torrent], grid_size: DataGridSize) {
    TabList::new("detail_tabs", &mut state.tab)
        .tab_icon(IconName::Info, TABS[0])
        .tab_icon(IconName::Folder, TABS[1])
        .tab_icon(IconName::Person, TABS[2])
        .tab_icon(IconName::Share, TABS[3])
        .tab_icon(IconName::ArrowSort, TABS[4])
        .tab_icon(IconName::Grid, TABS[5])
        .show(ui);
    ui.separator();
    let t = match selected {
        [] => return empty(ui, "Select a torrent to view details"),
        [one] => *one,
        many => return empty(ui, &format!("{} torrents selected", many.len())),
    };
    match state.tab {
        0 => {
            ScrollArea::vertical()
                .id_salt("overview")
                .show(ui, |ui| overview(ui, t))
                .inner
        }
        1 => files(ui, state, t),
        2 => peers(ui, &mut state.peers, t, grid_size),
        3 => trackers(ui, &mut state.trackers, t, grid_size),
        4 => speed(ui, t),
        _ => {
            ScrollArea::vertical()
                .id_salt("pieces")
                .show(ui, |ui| pieces(ui, t))
                .inner
        }
    }
}

fn empty(ui: &mut Ui, text: &str) {
    ui.centered_and_justified(|ui| {
        ui.label(rich(text, TypeRamp::Body1).weak());
    });
}

fn overview(ui: &mut Ui, t: &Torrent) {
    let theme = Theme::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(rich(&t.name, TypeRamp::Subtitle2));
        widgets::state_badge(ui, t.state);
    });
    ui.add_space(4.0);
    widgets::pieces_strip(ui, &t.pieces, 12.0);
    ui.add_space(6.0);
    let have = t.pieces.iter().filter(|p| **p == Piece::Have).count();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        ui.label(format!(
            "Progress {:.1}% ({}/{})",
            t.progress * 100.0,
            data::bytes(t.downloaded()),
            data::bytes(t.size)
        ));
        ui.label(format!("Down {}", nz(data::speed(t.down))));
        ui.label(format!(
            "Up {} ({} total)",
            nz(data::speed(t.up)),
            data::bytes(t.uploaded)
        ));
        ui.label(if t.progress >= 1.0 {
            "Complete".to_owned()
        } else {
            format!("ETA {}", data::eta(t.eta_secs()))
        });
        ui.label(format!("Ratio {:.2}", t.ratio));
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.label(format!(
            "Pieces {have}/{} ({} each)",
            t.pieces.len(),
            data::bytes(t.piece_size)
        ));
        ui.label("Peers");
        let live = t.peers + t.seeds;
        ui.label(
            RichText::new(format!("{live} live")).color(c(theme.color.status_success_foreground_1)),
        );
        ui.label(
            RichText::new(format!("{} connecting", live / 3))
                .color(c(theme.color.brand_foreground_1)),
        );
        ui.label(
            RichText::new(format!("{} queued", t.peers_total / 4))
                .color(c(theme.color.status_warning_foreground_1)),
        );
        ui.label(format!("{} seen", t.peers_total + t.seeds_total));
        ui.label(
            RichText::new(format!("{} dead", t.peers_total / 9))
                .color(c(theme.color.status_danger_foreground_1)),
        );
    });
    ui.add_space(8.0);
    Grid::new("overview_meta")
        .num_columns(4)
        .spacing([24.0, 6.0])
        .show(ui, |ui| {
            let key = |ui: &mut Ui, k: &str| {
                ui.label(RichText::new(k).color(c(theme.color.neutral_foreground_3)));
            };
            key(ui, "Hash");
            ui.label(RichText::new(&t.info_hash).monospace());
            key(ui, "Seeding time");
            ui.label(if t.seed_time_secs == 0 {
                "—".to_owned()
            } else {
                data::eta(Some(t.seed_time_secs))
            });
            ui.end_row();
            key(ui, "Output folder");
            ui.label(RichText::new(&t.save_path).monospace());
            key(ui, "Queue state");
            ui.label(match t.queue_position {
                Some(p) => format!("Queued #{p}"),
                None if t.state.is_active() => "Active".to_owned(),
                None => "—".to_owned(),
            });
            ui.end_row();
            key(ui, "Category");
            ui.label(if t.category.is_empty() {
                "None"
            } else {
                t.category
            });
            key(ui, "Sequential");
            ui.label(if t.sequential { "Yes" } else { "No" });
            ui.end_row();
            key(ui, "Added on");
            ui.label(t.added_on());
            key(ui, "Availability");
            ui.label(format!(
                "{:.2} min / {:.2} avg",
                t.availability.floor(),
                t.availability
            ));
            ui.end_row();
            if t.progress >= 1.0 {
                key(ui, "Super-seeding");
                ui.label(if t.super_seed { "Yes" } else { "No" });
                ui.end_row();
            }
        });
    if let Some(error) = t.error {
        ui.add_space(6.0);
        ui.label(RichText::new(error).color(c(theme.color.status_danger_foreground_1)));
    }
}

fn nz(s: String) -> String {
    if s.is_empty() { "0 B/s".to_owned() } else { s }
}

fn files(ui: &mut Ui, state: &mut DetailState, t: &Torrent) {
    if state.files_for != Some(t.id) {
        state.files_for = Some(t.id);
        state.files = TreeState::default();
        // Open the torrent's folder and its first child folder.
        state.files.expanded.insert(Vec::new());
        state.files.expanded.insert(vec![0]);
    }
    let root = &t.files;
    let label = |node: &FileNode| -> String {
        if node.children.is_empty() {
            format!(
                "{}  ({})  ·  {:.0}%  ·  {}",
                node.name,
                data::bytes(node.size),
                node.progress * 100.0,
                node.priority
            )
        } else {
            format!(
                "{}, {} files, {}",
                node.name,
                count_files(node),
                data::bytes(node.size)
            )
        }
    };
    ScrollArea::vertical()
        .id_salt("files")
        .auto_shrink(false)
        .show(ui, |ui| {
            TreeView::new(("files", t.id))
                .label("Files")
                .size(TreeSize::Small)
                .selection_mode(SelectionMode::Multi)
                .show(ui, &mut state.files, |parent, out| match parent {
                    None => out.push(
                        TreeNode::new(Vec::new(), label(root))
                            .icon(Icon::new(IconName::FolderOpen))
                            .children(Children::Some),
                    ),
                    Some(path) => {
                        if let Some(node) = root.at(path) {
                            for (i, child) in node.children.iter().enumerate() {
                                let mut key = path.clone();
                                key.push(i);
                                let folder = !child.children.is_empty();
                                let icon = if folder {
                                    IconName::Folder
                                } else if child.progress >= 1.0 {
                                    IconName::Checkmark
                                } else {
                                    IconName::Document
                                };
                                out.push(
                                    TreeNode::new(key, label(child))
                                        .icon(Icon::new(icon))
                                        .children(if folder {
                                            Children::Some
                                        } else {
                                            Children::None
                                        }),
                                );
                            }
                        }
                    }
                });
        });
}

fn count_files(node: &FileNode) -> usize {
    if node.children.is_empty() {
        1
    } else {
        node.children.iter().map(count_files).sum()
    }
}

fn sorted<T>(
    items: &[T],
    grid: &DataGridState,
    cmp: impl Fn(&T, &T, &str) -> Ordering,
) -> Vec<usize> {
    let mut rows: Vec<usize> = (0..items.len()).collect();
    if let Some(spec) = &grid.sort {
        rows.sort_by(|&a, &b| {
            let o = cmp(&items[a], &items[b], &spec.column);
            if spec.direction == SortDirection::Descending {
                o.reverse()
            } else {
                o
            }
        });
    }
    rows
}

fn peers(ui: &mut Ui, grid: &mut DataGridState, t: &Torrent, size: DataGridSize) {
    if !t.state.is_active() && t.state != State::Stalled {
        return empty(
            ui,
            "No live peer information available (torrent is not live)",
        );
    }
    if t.peer_list.is_empty() {
        return empty(ui, "No connected peers");
    }
    let cols = [
        Column::new("addr", "IP Address").width(ColumnWidth::Fixed(180.0)),
        Column::new("client", "Client").width(ColumnWidth::Fraction(1.0)),
        Column::new("flags", "Flags").width(ColumnWidth::Fixed(80.0)),
        Column::new("country", "Country").width(ColumnWidth::Fixed(70.0)),
        Column::new("progress", "Progress")
            .width(ColumnWidth::Fixed(80.0))
            .align(Align::End),
        Column::new("down", "Down Speed")
            .width(ColumnWidth::Fixed(100.0))
            .align(Align::End),
        Column::new("up", "Up Speed")
            .width(ColumnWidth::Fixed(100.0))
            .align(Align::End),
        Column::new("downloaded", "Downloaded")
            .width(ColumnWidth::Fixed(100.0))
            .align(Align::End),
        Column::new("uploaded", "Uploaded")
            .width(ColumnWidth::Fixed(100.0))
            .align(Align::End),
    ];
    let rows = sorted(&t.peer_list, grid, |a, b, col| {
        let f = |x: f64, y: f64| x.partial_cmp(&y).unwrap_or(Ordering::Equal);
        match col {
            "client" => a.client.cmp(&b.client),
            "flags" => a.flags.cmp(b.flags),
            "country" => a.country.cmp(b.country),
            "progress" => f(a.progress as f64, b.progress as f64),
            "down" => f(a.down, b.down),
            "up" => f(a.up, b.up),
            "downloaded" => a.downloaded.cmp(&b.downloaded),
            "uploaded" => a.uploaded.cmp(&b.uploaded),
            _ => a.addr.cmp(&b.addr),
        }
    });
    let theme = Theme::get(ui.ctx());
    DataGrid::new(("peers", t.id), &cols, rows.len())
        .label("Peers")
        .size(size)
        .selection_mode(SelectionMode::Single)
        .row_id(|i| RowId(rows[i] as u64))
        .show(ui, grid, |cell, ui| {
            let p = &t.peer_list[rows[cell.row]];
            let speed = |v: f64| {
                if v < 1.0 {
                    "-".to_owned()
                } else {
                    data::speed(v)
                }
            };
            match cell.column {
                "addr" => ui.label(RichText::new(&p.addr).monospace()),
                "client" => ui.label(&p.client),
                "flags" => ui.label(RichText::new(p.flags).monospace()),
                "country" => ui.label(p.country),
                "progress" => ui.label(format!("{:.0}%", p.progress * 100.0)),
                "down" => ui.label(
                    RichText::new(speed(p.down)).color(c(theme.color.status_success_foreground_1)),
                ),
                "up" => {
                    ui.label(RichText::new(speed(p.up)).color(c(theme.color.brand_foreground_1)))
                }
                "downloaded" => ui.label(data::bytes(p.downloaded)),
                _ => ui.label(data::bytes(p.uploaded)),
            };
        });
}

fn trackers(ui: &mut Ui, grid: &mut DataGridState, t: &Torrent, size: DataGridSize) {
    ui.horizontal(|ui| {
        ui.label("Info Hash:");
        ui.label(RichText::new(&t.info_hash).monospace());
    });
    if t.trackers.is_empty() {
        return empty(ui, "No trackers configured (DHT/PEX only)");
    }
    let cols = [
        Column::new("url", "Tracker")
            .width(ColumnWidth::Fraction(1.0))
            .min_width(240.0),
        Column::new("tier", "Tier")
            .width(ColumnWidth::Fixed(56.0))
            .align(Align::End),
        Column::new("status", "Status").width(ColumnWidth::Fixed(110.0)),
        Column::new("seeds", "Seeds")
            .width(ColumnWidth::Fixed(70.0))
            .align(Align::End),
        Column::new("peers", "Peers")
            .width(ColumnWidth::Fixed(70.0))
            .align(Align::End),
        Column::new("received", "Received")
            .width(ColumnWidth::Fixed(84.0))
            .align(Align::End),
        Column::new("announce", "Last Announce").width(ColumnWidth::Fixed(110.0)),
        Column::new("message", "Message").width(ColumnWidth::Fixed(200.0)),
    ];
    let rows = sorted(&t.trackers, grid, |a, b, col| match col {
        "tier" => a.tier.cmp(&b.tier),
        "status" => a.status.cmp(b.status),
        "seeds" => a.seeds.cmp(&b.seeds),
        "peers" => a.leechers.cmp(&b.leechers),
        "received" => a.downloaded.cmp(&b.downloaded),
        "announce" => a.next_announce.cmp(&b.next_announce),
        "message" => a.message.cmp(b.message),
        _ => a.url.cmp(&b.url),
    });
    DataGrid::new(("trackers", t.id), &cols, rows.len())
        .label("Trackers")
        .size(size)
        .selection_mode(SelectionMode::Single)
        .row_id(|i| RowId(rows[i] as u64))
        .show(ui, grid, |cell, ui| {
            let tr = &t.trackers[rows[cell.row]];
            match cell.column {
                "url" => {
                    ui.add(egui::Label::new(&tr.url).truncate());
                }
                "tier" => {
                    ui.label(tr.tier.to_string());
                }
                "status" => {
                    let color = if tr.status == "Working" {
                        fluent2_egui::BadgeColor::Success
                    } else {
                        fluent2_egui::BadgeColor::Danger
                    };
                    ui.add(
                        fluent2_egui::Badge::new(tr.status)
                            .color(color)
                            .size(fluent2_egui::BadgeSize::Small),
                    );
                }
                "seeds" => {
                    ui.label(tr.seeds.to_string());
                }
                "peers" => {
                    ui.label(tr.leechers.to_string());
                }
                "received" => {
                    ui.label(tr.downloaded.to_string());
                }
                "announce" => {
                    let s = 1800 - tr.next_announce.min(1800);
                    ui.label(match s {
                        0..=4 => "just now".to_owned(),
                        5..=59 => format!("{s}s ago"),
                        _ => format!("{}m ago", s / 60),
                    });
                }
                _ => {
                    ui.add(egui::Label::new(tr.message).truncate());
                }
            }
        });
}

fn speed(ui: &mut Ui, t: &Torrent) {
    if !matches!(t.state, State::Downloading | State::Seeding) {
        return empty(ui, "Speed chart is only available for active torrents.");
    }
    let down: Vec<f32> = t.down_history.iter().copied().collect();
    let up: Vec<f32> = t.up_history.iter().copied().collect();
    let width = ui.available_width();
    let height = ((ui.available_height() - 60.0) / 2.0).clamp(40.0, 160.0);
    for (name, series, now) in [("Download", &down, t.down), ("Upload", &up, t.up)] {
        ui.horizontal(|ui| {
            ui.label(rich(name, TypeRamp::Body1Strong));
            ui.label(nz(data::speed(now)));
            ui.label(
                RichText::new(format!(
                    "peak {}",
                    nz(data::speed(
                        series.iter().copied().fold(0.0, f32::max) as f64
                    ))
                ))
                .weak(),
            );
        });
        ui.add(
            Sparkline::new(series)
                .range(0.0, series.iter().copied().fold(1.0f32, f32::max) * 1.15)
                .size(vec2(width, height))
                .last_dot(true)
                .label(format!("{name} speed, last {} seconds", series.len())),
        );
        ui.add_space(6.0);
    }
}

fn pieces(ui: &mut Ui, t: &Torrent) {
    let have = t.pieces.iter().filter(|p| **p == Piece::Have).count();
    let busy = t
        .pieces
        .iter()
        .filter(|p| **p == Piece::Downloading)
        .count();
    ui.horizontal(|ui| {
        ui.label(rich(
            format!("{} pieces of {}", t.pieces.len(), data::bytes(t.piece_size)),
            TypeRamp::Body1Strong,
        ));
        ui.label(format!(
            "{have} verified · {busy} downloading · {} missing",
            t.pieces.len() - have - busy
        ));
    });
    widgets::pieces_legend(ui);
    ui.add_space(4.0);
    widgets::pieces_map(ui, &t.pieces, t.piece_size);
}
