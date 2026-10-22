//! The torrent table: a fluent2 `DataGrid` with sortable columns, multi-select and a context
//! menu, over the rows the sidebar and search leave visible.

use std::cmp::Ordering;

use egui::{Color32, RichText, Ui};
use fluent2_egui::data::data_grid::{Align, Column, ColumnWidth, SortDirection};
use fluent2_egui::theme::{Theme, c};
use fluent2_egui::{DataGrid, DataGridSize, DataGridState, RowId, SelectionMode};
use fluent2_egui::{Icon, IconName, IconSize};

use crate::data::{self, Torrent};
use crate::widgets;

/// Column ids and headers, in display order, as the web UI's compact table has them
/// (`stores/columnStore.ts`). The checkbox column comes from the grid's multi-select mode.
pub const COLUMNS: [(&str, &str); 20] = [
    ("status_icon", ""),
    ("id", "ID"),
    ("name", "Name"),
    ("size", "Size"),
    ("progress", "Progress"),
    ("recv", "Recv"),
    ("down", "↓ Speed"),
    ("up", "↑ Speed"),
    ("sent", "Sent"),
    ("eta", "ETA"),
    ("peers", "Peers"),
    ("state", "State"),
    ("info_hash", "Info Hash"),
    ("ratio", "Ratio"),
    ("category", "Category"),
    ("seed_time", "Seed Time"),
    ("queue", "Queue"),
    ("seq", "Seq"),
    ("avail", "Avail"),
    ("added", "Added On"),
];

/// Columns shown by default; the rest are switched on from the column menu.
pub const DEFAULT_VISIBLE: [&str; 14] = [
    "status_icon",
    "id",
    "name",
    "size",
    "progress",
    "recv",
    "down",
    "up",
    "sent",
    "eta",
    "peers",
    "state",
    "ratio",
    "added",
];

fn column(id: &'static str, header: &'static str) -> Column {
    let col = Column::new(id, header);
    let fixed = |w: f32| ColumnWidth::Fixed(w);
    match id {
        "status_icon" => col
            .width(fixed(36.0))
            .sortable(false)
            .resizable(false)
            .align(Align::Center),
        "id" => col.width(fixed(56.0)).align(Align::End),
        "name" => col.width(ColumnWidth::Fraction(1.0)).min_width(200.0),
        "progress" => col.width(fixed(140.0)),
        "state" => col.width(fixed(120.0)),
        "size" | "recv" | "sent" => col.width(fixed(88.0)).align(Align::End),
        "down" | "up" => col.width(fixed(96.0)).align(Align::End),
        "eta" => col.width(fixed(80.0)).align(Align::End),
        "peers" => col.width(fixed(80.0)).align(Align::End),
        "ratio" => col.width(fixed(64.0)).align(Align::End),
        "info_hash" => col.width(fixed(120.0)).sortable(false),
        "seq" => col.width(fixed(52.0)).sortable(false).align(Align::Center),
        "queue" => col.width(fixed(64.0)),
        "seed_time" | "avail" => col.width(fixed(84.0)).align(Align::End),
        "added" => col.width(fixed(130.0)),
        _ => col.width(fixed(110.0)),
    }
}

pub fn columns(visible: &[&'static str]) -> Vec<Column> {
    COLUMNS
        .iter()
        .filter(|(id, _)| visible.contains(id))
        .map(|(id, header)| column(id, header))
        .collect()
}

fn compare(a: &Torrent, b: &Torrent, column: &str) -> Ordering {
    let f = |x: f64, y: f64| x.partial_cmp(&y).unwrap_or(Ordering::Equal);
    match column {
        "name" => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        "size" => a.size.cmp(&b.size),
        "progress" => f(a.progress as f64, b.progress as f64),
        "recv" => a.downloaded().cmp(&b.downloaded()),
        "down" => f(a.down, b.down),
        "up" => f(a.up, b.up),
        "sent" => a.uploaded.cmp(&b.uploaded),
        "eta" => a
            .eta_secs()
            .unwrap_or(u64::MAX)
            .cmp(&b.eta_secs().unwrap_or(u64::MAX)),
        "peers" => a.peers.cmp(&b.peers),
        "state" => a.state.cmp(&b.state),
        "ratio" => f(a.ratio as f64, b.ratio as f64),
        "category" => a.category.cmp(b.category),
        "seed_time" => a.seed_time_secs.cmp(&b.seed_time_secs),
        "queue" => a
            .queue_position
            .unwrap_or(u32::MAX)
            .cmp(&b.queue_position.unwrap_or(u32::MAX)),
        "avail" => f(a.availability as f64, b.availability as f64),
        "added" => (b.added_days_ago, 1440 - b.added_minute)
            .cmp(&(a.added_days_ago, 1440 - a.added_minute)),
        _ => a.id.cmp(&b.id),
    }
}

/// Sort `rows` (indices into `torrents`) by the grid's sort spec.
pub fn sort(rows: &mut [usize], torrents: &[Torrent], state: &DataGridState) {
    let Some(spec) = &state.sort else { return };
    rows.sort_by(|&a, &b| {
        let ord = compare(&torrents[a], &torrents[b], &spec.column);
        match spec.direction {
            SortDirection::Ascending => ord,
            SortDirection::Descending => ord.reverse(),
        }
    });
}

pub struct TableOutput {
    pub response: fluent2_egui::DataGridResponse,
}

pub fn show(
    ui: &mut Ui,
    torrents: &[Torrent],
    rows: &[usize],
    visible: &[&'static str],
    state: &mut DataGridState,
    size: DataGridSize,
) -> TableOutput {
    let cols = columns(visible);
    let response = DataGrid::new("torrents", &cols, rows.len())
        .label("Torrents")
        .size(size)
        .selection_mode(SelectionMode::Multi)
        .row_id(|i| RowId(torrents[rows[i]].id))
        .show(ui, state, |cell, ui| {
            let t = &torrents[rows[cell.row]];
            let dash = || "—".to_owned();
            match cell.column {
                "status_icon" => {
                    let (icon, color) = status_icon(ui, t);
                    ui.add(Icon::new(icon).size(IconSize::S16).tint(color));
                }
                "id" => {
                    ui.label(RichText::new(t.id.to_string()).monospace().weak());
                }
                "name" => match t.error {
                    None => {
                        ui.add(egui::Label::new(&t.name).truncate())
                            .on_hover_text(&t.name);
                    }
                    Some(error) => {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.add(egui::Label::new(&t.name).truncate())
                                .on_hover_text(&t.name);
                            let red = c(Theme::get(ui.ctx()).color.status_danger_foreground_1);
                            ui.add(
                                egui::Label::new(RichText::new(error).small().color(red))
                                    .truncate(),
                            );
                        });
                    }
                },
                "size" => {
                    ui.label(data::bytes(t.size));
                }
                "progress" => widgets::progress_cell(ui, t.progress, t.state),
                "recv" => {
                    ui.label(data::bytes(t.downloaded()));
                }
                "down" => speed_label(ui, t.down, true),
                "up" => speed_label(ui, t.up, false),
                "sent" => {
                    ui.label(if t.uploaded == 0 {
                        String::new()
                    } else {
                        data::bytes(t.uploaded)
                    });
                }
                "eta" => {
                    ui.label(if t.progress >= 1.0 {
                        "Done".to_owned()
                    } else if t.state == data::State::Downloading {
                        data::eta(t.eta_secs())
                    } else {
                        "N/A".to_owned()
                    });
                }
                "peers" => {
                    ui.label(format!(
                        "{}/{}",
                        t.peers + t.seeds,
                        t.peers_total + t.seeds_total
                    ));
                }
                "state" => {
                    widgets::state_badge(ui, t.state);
                }
                "info_hash" => {
                    ui.add(
                        egui::Label::new(RichText::new(&t.info_hash).monospace().small())
                            .truncate(),
                    );
                }
                "ratio" => {
                    ui.label(format!("{:.2}", t.ratio));
                }
                "category" => {
                    ui.label(if t.category.is_empty() {
                        dash()
                    } else {
                        t.category.to_owned()
                    });
                }
                "seed_time" => {
                    ui.label(if t.seed_time_secs == 0 {
                        dash()
                    } else {
                        data::eta(Some(t.seed_time_secs))
                    });
                }
                "queue" => {
                    ui.label(match t.queue_position {
                        Some(p) => format!("#{p}"),
                        None if t.state.is_active() => "Active".to_owned(),
                        None => dash(),
                    });
                }
                "seq" => {
                    ui.label(if t.sequential { "✓" } else { "—" });
                }
                "avail" => {
                    ui.label(if t.availability > 0.0 {
                        format!("{:.1}", t.availability)
                    } else {
                        dash()
                    });
                }
                _ => {
                    ui.label(t.added_on());
                }
            }
        });
    TableOutput { response }
}

fn status_icon(ui: &Ui, t: &Torrent) -> (IconName, Color32) {
    let theme = Theme::get(ui.ctx());
    match t.state {
        data::State::Error => (
            IconName::ErrorCircle,
            c(theme.color.status_danger_foreground_1),
        ),
        data::State::Queued => (
            IconName::MoreHorizontal,
            c(theme.color.status_warning_foreground_1),
        ),
        data::State::Seeding => (
            IconName::ArrowUp,
            c(theme.color.status_success_foreground_1),
        ),
        _ if t.progress >= 1.0 => (
            IconName::Checkmark,
            c(theme.color.status_success_foreground_1),
        ),
        data::State::Downloading | data::State::Checking => {
            (IconName::ArrowDownload, c(theme.color.brand_foreground_1))
        }
        _ => (IconName::Subtract, c(theme.color.neutral_foreground_3)),
    }
}

/// Speed cell: "-" at zero, accent-coloured and medium weight when moving.
fn speed_label(ui: &mut Ui, bps: f64, down: bool) {
    if bps < 0.01 {
        ui.label(RichText::new("-").weak());
        return;
    }
    let theme = Theme::get(ui.ctx());
    let color = if down {
        c(theme.color.brand_foreground_1)
    } else {
        c(theme.color.status_success_foreground_1)
    };
    ui.label(RichText::new(data::speed(bps)).color(color).strong());
}
