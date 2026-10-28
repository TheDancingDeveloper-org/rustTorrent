//! Main screen: a hand-laid filter sidebar and a virtualised torrent grid.
//!
//! fluent2-egui has no navigation view and no split view (design §6), so the
//! sidebar is an egui `SidePanel`. The grid is `data::DataGrid`, which
//! virtualises, so this screen does not cap the list.

use fluent2_egui::controls::{Appearance, Button};
use fluent2_egui::data::data_grid::{Align, Column, ColumnWidth};
use fluent2_egui::data::{DataGrid, DataGridSize, DataGridState, RowId, SelectionMode};
use rtbit_client::Torrent;

use crate::format;
use crate::state::{ANY, ListCache, StatusFilter};

/// Draw the sidebar and the grid into `ui`.
pub fn draw_main(
    ui: &mut egui::Ui,
    cache: &ListCache,
    status: &mut StatusFilter,
    category: &mut String,
    tracker: &mut String,
    grid: &mut DataGridState,
) {
    keep_choice(category, &cache.categories());
    keep_choice(tracker, &cache.trackers());

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
        grid_panel(ui, cache, *status, category, tracker, grid);
    });
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
) {
    let rows = cache.filtered(status, category, tracker);
    if rows.is_empty() {
        ui.add_space(24.0);
        ui.heading("Torrents");
        ui.label(if cache.torrents.is_empty() {
            "No torrents."
        } else {
            "No torrents match the filter."
        });
        return;
    }
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
            ui.label(cell_text(torrent, cell.column));
        });
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
