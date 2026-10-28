//! Main screen: a hand-laid filter sidebar and a virtualised torrent grid.
//!
//! fluent2-egui has no navigation view and no split view (design §6), so the
//! sidebar is an egui `SidePanel`. The grid is `data::DataGrid`, which
//! virtualises, so this screen does not cap the list.

use fluent2_egui::controls::{Appearance, Button};
use fluent2_egui::data::data_grid::{Align, Column, ColumnWidth};
use fluent2_egui::data::{DataGrid, DataGridSize, DataGridState, RowId, SelectionMode};
use fluent2_egui::office::command_bar::{CommandBar, CommandItem};
use fluent2_egui::surfaces::{CloseReason, Dialog, DialogResult};
use fluent2_icons::Icon;
use rtbit_client::Torrent;

use crate::format;
use crate::state::{ANY, ListCache, StatusFilter};

/// What the command bar asked for this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct MainAction {
    pub selected: Option<u64>,
    pub action: Option<RowAction>,
}

/// A row command the daemon understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    Start,
    Pause,
    Delete,
}

/// Row selection and the delete confirmation, owned by the shell.
pub struct RowActions<'a> {
    /// Torrent the command bar acts on.
    pub selected: Option<u64>,
    /// Whether the delete dialog is open.
    pub confirm_delete: &'a mut bool,
    /// An action is already in flight, so the bar ignores clicks.
    pub busy: bool,
}

/// Draw the sidebar and the grid into `ui`.
pub fn draw_main(
    ui: &mut egui::Ui,
    cache: &ListCache,
    status: &mut StatusFilter,
    category: &mut String,
    tracker: &mut String,
    grid: &mut DataGridState,
    rows: RowActions<'_>,
) -> MainAction {
    keep_choice(category, &cache.categories());
    keep_choice(tracker, &cache.trackers());

    let mut out = MainAction::default();
    let items = [
        CommandItem {
            key: "start",
            icon: Icon::ArrowClockwise,
            label: "Start",
            priority: 90,
        },
        CommandItem {
            key: "pause",
            icon: Icon::Dismiss,
            label: "Pause",
            priority: 80,
        },
        CommandItem {
            key: "delete",
            icon: Icon::Delete,
            label: "Delete",
            priority: 70,
        },
    ];
    let bar = CommandBar::new("rtbit_actions", &items).show(ui);
    if !rows.busy && rows.selected.is_some() {
        match bar.activated {
            Some("start") => out.action = Some(RowAction::Start),
            Some("pause") => out.action = Some(RowAction::Pause),
            Some("delete") => *rows.confirm_delete = true,
            _ => {}
        }
    }

    egui::Panel::left("rtbit_sidebar")
        .resizable(false)
        .exact_size(196.0)
        .show(ui, |ui| {
            sidebar(ui, cache, status, category, tracker);
        });
    egui::CentralPanel::default_margins().show(ui, |ui| {
        if let Some(error) = &cache.error {
            ui.colored_label(egui::Color32::from_rgb(196, 43, 28), error);
            ui.add_space(4.0);
        }
        let picked = grid_panel(ui, cache, *status, category, tracker, grid);
        if picked.is_some() {
            out.selected = picked;
        }
    });

    if *rows.confirm_delete {
        let result = Dialog::new("rtbit_delete")
            .title("Delete torrent")
            .actions(|actions| {
                actions.primary("Delete");
                actions.secondary("Cancel");
            })
            .show(ui.ctx(), rows.confirm_delete, |ui| {
                ui.label("Delete this torrent and its files?");
            });
        if let DialogResult::Closed(CloseReason::Button("Delete")) = result {
            out.action = Some(RowAction::Delete);
        }
    }
    out
}

fn sidebar(
    ui: &mut egui::Ui,
    cache: &ListCache,
    status: &mut StatusFilter,
    category: &mut String,
    tracker: &mut String,
) {
    ui.add_space(8.0);
    ui.label("Status");
    for filter in StatusFilter::ALL {
        let on = *status == filter;
        if ui
            .add(Button::new(filter.label()).appearance(if on {
                Appearance::Primary
            } else {
                Appearance::Secondary
            }))
            .clicked()
        {
            *status = filter;
        }
    }
    ui.add_space(12.0);
    choice(ui, "Category", category, &cache.categories());
    ui.add_space(8.0);
    choice(ui, "Tracker", tracker, &cache.trackers());
    ui.add_space(12.0);
    speeds(ui, cache);
}

fn choice(ui: &mut egui::Ui, label: &str, current: &mut String, options: &[String]) {
    ui.label(label);
    for option in options {
        let on = current == option;
        if ui
            .add(Button::new(option).appearance(if on {
                Appearance::Primary
            } else {
                Appearance::Secondary
            }))
            .clicked()
        {
            *current = option.clone();
        }
    }
}

fn speeds(ui: &mut egui::Ui, cache: &ListCache) {
    ui.label("Speed");
    match &cache.stats {
        Some(stats) => {
            ui.label(format!("Down {}", format::speed(&stats.download_speed)));
            ui.label(format!("Up {}", format::speed(&stats.upload_speed)));
        }
        None => {
            ui.label("Down —");
            ui.label("Up —");
        }
    }
}

fn grid_panel(
    ui: &mut egui::Ui,
    cache: &ListCache,
    status: StatusFilter,
    category: &str,
    tracker: &str,
    grid: &mut DataGridState,
) -> Option<u64> {
    let rows = cache.filtered(status, category, tracker);
    if rows.is_empty() {
        ui.add_space(24.0);
        ui.heading("Torrents");
        ui.label(if cache.torrents.is_empty() {
            "No torrents."
        } else {
            "No torrents match the filter."
        });
        return None;
    }
    let mut pending: Option<u64> = None;
    let columns = [
        Column::new("name", "Name").width(ColumnWidth::Fraction(3.0)),
        Column::new("size", "Size")
            .width(ColumnWidth::Fixed(140.0))
            .align(Align::End),
        Column::new("state", "State").width(ColumnWidth::Fixed(110.0)),
        Column::new("down", "Down")
            .width(ColumnWidth::Fixed(110.0))
            .align(Align::End),
        Column::new("up", "Up")
            .width(ColumnWidth::Fixed(110.0))
            .align(Align::End),
        Column::new("category", "Category").width(ColumnWidth::Fixed(120.0)),
    ];
    DataGrid::new("rtbit_torrents", &columns, rows.len())
        .label("Torrents")
        .size(DataGridSize::Small)
        .selection_mode(SelectionMode::Single)
        .row_id(|index| RowId(rows[index].id.unwrap_or(index as u64)))
        .show(ui, grid, |cell, ui| {
            let torrent = rows[cell.row];
            let text = cell_text(torrent, cell.column);
            if cell.column == "name" {
                if ui.selectable_label(cell.selected, text).clicked()
                    && let Some(id) = torrent.id
                {
                    pending = Some(id);
                }
            } else {
                ui.label(text);
            }
        });
    if let Some(id) = pending {
        grid.selection.selected.clear();
        grid.selection.selected.insert(RowId(id));
    }
    grid.selection.selected.iter().next().map(|row| row.0)
}

fn cell_text(torrent: &Torrent, column: &str) -> String {
    let stats = torrent.stats.as_ref();
    match column {
        "name" => torrent
            .name
            .clone()
            .unwrap_or_else(|| torrent.info_hash.clone()),
        "size" => format::progress(
            stats.map(|s| s.progress_bytes).unwrap_or(0),
            stats.map(|s| s.total_bytes).unwrap_or(0),
        ),
        "state" => format::state_label(stats.and_then(|s| s.state)),
        "down" => stats
            .and_then(|s| s.live.as_ref())
            .map(|live| format::speed(&live.download_speed))
            .unwrap_or_else(|| "—".to_owned()),
        "up" => stats
            .and_then(|s| s.live.as_ref())
            .map(|live| format::speed(&live.upload_speed))
            .unwrap_or_else(|| "—".to_owned()),
        "category" => torrent.category.clone().unwrap_or_else(|| "—".to_owned()),
        _ => String::new(),
    }
}

/// Drop a selected category or tracker that the latest poll no longer has.
fn keep_choice(current: &mut String, options: &[String]) {
    if current != ANY && !options.iter().any(|option| option == current) {
        *current = ANY.to_owned();
    }
}
