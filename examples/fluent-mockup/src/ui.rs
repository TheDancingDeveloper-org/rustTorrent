//! Toolbar, sidebar, torrent grid and a detail pane, all from fake data.

use fluent2_egui::IconName;
use fluent2_egui::controls::{Appearance, Button};
use fluent2_egui::data::data_grid::{Align, Column, ColumnWidth};
use fluent2_egui::data::nav_view::{NavState, NavigationView};
use fluent2_egui::data::{DataGrid, DataGridSize, DataGridState, RowId, SelectionMode};
use fluent2_egui::office::command_bar::{CommandBar, CommandItem};
use fluent2_egui::surfaces::TabList;
use fluent2_egui::theme::{Theme, install};
use fluent2_egui::tokens::FLUENT;

use crate::data::{self, Torrent};

const FILTERS: [&str; 4] = ["all", "downloading", "seeding", "paused"];

/// The whole mockup. Owns the grid selection and which detail tab is open.
pub struct Mockup {
    torrents: Vec<Torrent>,
    filter: &'static str,
    grid: DataGridState,
    selected: Option<u64>,
    tab: usize,
    note: String,
}

impl Default for Mockup {
    fn default() -> Self {
        Self::new()
    }
}

impl Mockup {
    pub fn new() -> Self {
        Self {
            torrents: data::torrents(),
            filter: "all",
            grid: DataGridState::default(),
            selected: Some(1),
            tab: 0,
            note: String::new(),
        }
    }

    /// Draw one frame. `time` drives the small speed wobble.
    pub fn ui(&mut self, ui: &mut egui::Ui, time: f32) {
        self.toolbar(ui);
        if !self.note.is_empty() {
            ui.label(&self.note);
        }
        let mut nav = NavState {
            selected: Some(self.filter.to_owned()),
            ..NavState::default()
        };
        let view = NavigationView::new("mock_nav")
            .title("Torrents")
            .item("all", IconName::Home, "All")
            .item("downloading", IconName::ArrowClockwise, "Downloading")
            .item("seeding", IconName::Checkmark, "Seeding")
            .item("paused", IconName::Dismiss, "Paused");
        view.show(ui, &mut nav, |ui| {
            self.grid(ui, time);
            ui.separator();
            self.detail(ui, time);
        });
        if let Some(key) = nav.selected.as_deref()
            && let Some(filter) = FILTERS.into_iter().find(|one| *one == key)
        {
            self.filter = filter;
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let items = [
            CommandItem {
                key: "add",
                icon: IconName::Add,
                label: "Add",
                priority: 100,
            },
            CommandItem {
                key: "start",
                icon: IconName::ArrowClockwise,
                label: "Start",
                priority: 90,
            },
            CommandItem {
                key: "pause",
                icon: IconName::Dismiss,
                label: "Pause",
                priority: 80,
            },
            CommandItem {
                key: "delete",
                icon: IconName::Delete,
                label: "Delete",
                priority: 70,
            },
        ];
        let bar = CommandBar::new("mock_toolbar", &items).show(ui);
        if let Some(key) = bar.activated {
            self.note = match key {
                "add" => "Add is a mock. Nothing is opened.".to_owned(),
                "start" => self.act("started"),
                "pause" => self.act("paused"),
                "delete" => self.act("would be deleted"),
                _ => String::new(),
            };
        }
    }

    fn act(&self, verb: &str) -> String {
        match self.current() {
            Some(torrent) => format!("{} {verb}.", torrent.name),
            None => format!("Select a torrent to {verb} it."),
        }
    }

    fn visible(&self) -> Vec<&Torrent> {
        self.torrents
            .iter()
            .filter(|torrent| self.filter == "all" || torrent.state == self.filter)
            .collect()
    }

    fn grid(&mut self, ui: &mut egui::Ui, time: f32) {
        let rows = self.visible();
        if rows.is_empty() {
            ui.label("No torrents match the filter.");
            return;
        }
        let columns = [
            Column::new("name", "Name").width(ColumnWidth::Fraction(3.0)),
            Column::new("size", "Size")
                .width(ColumnWidth::Fixed(110.0))
                .align(Align::End),
            Column::new("progress", "Progress")
                .width(ColumnWidth::Fixed(90.0))
                .align(Align::End),
            Column::new("state", "State").width(ColumnWidth::Fixed(120.0)),
            Column::new("down", "Down")
                .width(ColumnWidth::Fixed(100.0))
                .align(Align::End),
            Column::new("up", "Up")
                .width(ColumnWidth::Fixed(100.0))
                .align(Align::End),
            Column::new("peers", "Peers")
                .width(ColumnWidth::Fixed(70.0))
                .align(Align::End),
            Column::new("category", "Category").width(ColumnWidth::Fixed(100.0)),
        ];
        // Copy the visible rows first. `DataGrid::show` mutably borrows the
        // grid state, so the cell closure cannot also borrow `self`.
        let shown: Vec<Torrent> = rows.into_iter().cloned().collect();
        let mut picked = None;
        DataGrid::new("mock_torrents", &columns, shown.len())
            .label("Torrents")
            .size(DataGridSize::Small)
            .selection_mode(SelectionMode::Single)
            .row_id(|index| RowId(shown[index].id))
            .show(ui, &mut self.grid, |cell, ui| {
                let torrent = &shown[cell.row];
                let text = cell_text(torrent, cell.column, time);
                if cell.column == "name" {
                    if ui.selectable_label(cell.selected, text).clicked() {
                        picked = Some(torrent.id);
                    }
                } else {
                    ui.label(text);
                }
            });
        if let Some(id) = picked {
            self.grid.selection.selected.clear();
            self.grid.selection.selected.insert(RowId(id));
            self.selected = Some(id);
        }
    }

    fn current(&self) -> Option<&Torrent> {
        let id = self.selected?;
        self.torrents.iter().find(|torrent| torrent.id == id)
    }

    fn detail(&mut self, ui: &mut egui::Ui, time: f32) {
        let Some(torrent) = self.current().cloned() else {
            ui.label("Select a torrent.");
            return;
        };
        ui.heading(&torrent.name);
        ui.label(format!(
            "{} · {} · {}",
            torrent.category, torrent.tracker, torrent.state
        ));
        let mut tabs = TabList::new("mock_detail", &mut self.tab);
        for label in ["Overview", "Files", "Peers", "Speed", "Pieces"] {
            tabs = tabs.tab(label);
        }
        tabs.show(ui);
        ui.add_space(6.0);
        match self.tab {
            0 => overview(ui, &torrent, time),
            1 => files(ui, &torrent),
            2 => peers(ui, &torrent),
            3 => speed(ui, &torrent, time),
            _ => pieces(ui, &torrent),
        }
    }
}

fn cell_text(torrent: &Torrent, column: &str, time: f32) -> String {
    match column {
        "name" => torrent.name.clone(),
        "size" => data::size(torrent.size),
        "progress" => format!("{:.0}%", shown_progress(torrent, time) * 100.0),
        "state" => torrent.state.to_owned(),
        "down" => data::rate(wobble(torrent.download_bps, time)),
        "up" => data::rate(wobble(torrent.upload_bps, time)),
        "peers" => torrent.peers.to_string(),
        "category" => torrent.category.to_owned(),
        _ => String::new(),
    }
}

fn shown_progress(torrent: &Torrent, time: f32) -> f32 {
    if torrent.progress >= 1.0 || torrent.state != "downloading" {
        return torrent.progress;
    }
    (torrent.progress + (time * 0.01).fract() * 0.02).min(0.99)
}

fn wobble(base: u64, time: f32) -> u64 {
    if base == 0 {
        return 0;
    }
    let factor = 0.85 + (time * 0.7 + base as f32 * 0.001).sin().abs() * 0.3;
    (base as f32 * factor) as u64
}

fn overview(ui: &mut egui::Ui, torrent: &Torrent, time: f32) {
    ui.label(format!("Size {}", data::size(torrent.size)));
    ui.label(format!(
        "Progress {:.0}%",
        shown_progress(torrent, time) * 100.0
    ));
    ui.label(format!(
        "Down {}   Up {}",
        data::rate(wobble(torrent.download_bps, time)),
        data::rate(wobble(torrent.upload_bps, time))
    ));
    ui.label(format!("Peers {}   Seeds {}", torrent.peers, torrent.seeds));
    ui.label("Save path  /data/downloads (fake)");
}

fn files(ui: &mut egui::Ui, torrent: &Torrent) {
    for index in 0..4 {
        let included = index != 3;
        let label = format!(
            "{}  {}-{:02}.bin",
            if included { "[x]" } else { "[ ]" },
            torrent.category.to_lowercase(),
            index + 1
        );
        let _ = ui.add(Button::new(label).appearance(Appearance::Subtle));
    }
    ui.label("File priority and selection are not wired. This is a mockup.");
}

fn peers(ui: &mut egui::Ui, torrent: &Torrent) {
    ui.label("Address            Client     Down     Up");
    for index in 0..6 {
        let host = format!("10.0.{}.{}", torrent.id, 10 + index);
        ui.label(format!(
            "{host:<18}  mock-{:02}   {}   {}",
            index,
            data::rate(80_000 + index as u64 * 20_000),
            data::rate(20_000 + index as u64 * 4_000)
        ));
    }
}

fn speed(ui: &mut egui::Ui, torrent: &Torrent, time: f32) {
    ui.label("Download");
    spark(
        ui,
        torrent.download_bps,
        time,
        egui::Color32::from_rgb(0, 120, 212),
    );
    ui.label("Upload");
    spark(
        ui,
        torrent.upload_bps,
        time,
        egui::Color32::from_rgb(16, 124, 16),
    );
}

fn spark(ui: &mut egui::Ui, base: u64, time: f32, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(280.0, 48.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, egui::Color32::from_gray(32));
    if base == 0 {
        return;
    }
    let mut points = Vec::with_capacity(24);
    for step in 0..24 {
        let sample = wobble(base, time - step as f32 * 0.15) as f32 / base as f32;
        let x = rect.right() - step as f32 * (rect.width() / 23.0);
        let y = rect.bottom() - sample.clamp(0.0, 1.4) / 1.4 * rect.height();
        points.push(egui::pos2(x, y));
    }
    painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, color)));
}

fn pieces(ui: &mut egui::Ui, torrent: &Torrent) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(320.0, 36.0), egui::Sense::hover());
    let painter = ui.painter();
    let width = rect.width() / torrent.pieces.len() as f32;
    for (index, have) in torrent.pieces.iter().enumerate() {
        let piece = egui::Rect::from_min_size(
            egui::pos2(rect.left() + index as f32 * width, rect.top()),
            egui::vec2(width - 1.0, rect.height()),
        );
        let color = if *have {
            egui::Color32::from_rgb(16, 124, 16)
        } else {
            egui::Color32::from_gray(70)
        };
        painter.rect_filled(piece, 0.0, color);
    }
    let have = torrent.pieces.iter().filter(|bit| **bit).count();
    ui.label(format!("{have} of {} pieces (fake)", torrent.pieces.len()));
}

/// Install the light fluent2 theme on a context.
pub fn install_theme(ctx: &egui::Context) {
    install(ctx, &Theme::light(FLUENT));
}
