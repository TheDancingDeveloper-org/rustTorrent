//! Main screen: a navigation pane for status and a virtualised torrent grid.
//!
//! Status is `data::NavigationView`. Category, tracker and the speed readout
//! stay as buttons under the pane: their labels come from the daemon, and a
//! navigation item key has to be a static string. The grid is `data::DataGrid`,
//! which virtualises, so this screen does not cap the list.

use fluent2_egui::IconName;
use fluent2_egui::controls::{Appearance, Button, Checkbox, Input};
use fluent2_egui::data::data_grid::{Align, Column, ColumnWidth};
use fluent2_egui::data::nav_view::{NavState, NavigationView};
use fluent2_egui::data::{DataGrid, DataGridSize, DataGridState, RowId, SelectionMode};
use fluent2_egui::office::command_bar::{CommandBar, CommandItem};
use fluent2_egui::surfaces::{CloseReason, Dialog, DialogResult};
use rtbit_client::Torrent;

use crate::format;
use crate::state::{ANY, ListCache, StatusFilter};

/// What the command bar or the add dialog asked for this frame.
#[derive(Clone, Debug, Default)]
pub struct MainAction {
    pub selected: Option<u64>,
    pub action: Option<RowAction>,
    pub add: Option<AddRequest>,
}

/// A torrent the user wants the daemon to add.
///
/// Either a magnet link or the bytes of a `.torrent` file, plus the save path,
/// an optional category and whether to start paused. The dialog has already
/// checked the magnet and the path; the daemon checks them again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddRequest {
    pub source: AddSource,
    pub save_path: String,
    pub category: String,
    pub start_paused: bool,
}

/// Where the torrent comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddSource {
    /// A magnet link, already checked as `magnet:?xt=urn:btih:`.
    Magnet(String),
    /// Bytes of a `.torrent` file the user picked.
    File { name: String, bytes: Vec<u8> },
}

/// A row command the daemon understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    Start,
    Pause,
    Delete,
}

/// Row selection, the delete confirmation and the add dialog, owned by the shell.
pub struct RowActions<'a> {
    /// Torrent the command bar acts on.
    pub selected: Option<u64>,
    /// Whether the delete dialog is open.
    pub confirm_delete: &'a mut bool,
    /// Whether the add dialog is open.
    pub add_open: &'a mut bool,
    /// Fields the add dialog edits.
    pub draft: &'a mut AddDraft,
    /// An action is already in flight, so the bar ignores clicks.
    pub busy: bool,
}

/// Fields of the add dialog. The shell owns them so a rebuild keeps the text.
#[derive(Clone, Debug, Default)]
pub struct AddDraft {
    pub magnet: String,
    pub file_name: String,
    pub file_bytes: Vec<u8>,
    pub save_path: String,
    pub category: String,
    pub start_paused: bool,
    /// Shown in the dialog when the magnet or the path is not usable.
    pub error: String,
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
    let bar = CommandBar::new("rtbit_actions", &items).show(ui);
    if !rows.busy {
        if bar.activated == Some("add") {
            *rows.add_open = true;
        }
        if rows.selected.is_some() {
            match bar.activated {
                Some("start") => out.action = Some(RowAction::Start),
                Some("pause") => out.action = Some(RowAction::Pause),
                Some("delete") => *rows.confirm_delete = true,
                _ => {}
            }
        }
    }

    let mut nav = NavState {
        selected: Some(status_key(*status).to_owned()),
        ..NavState::default()
    };
    let view = NavigationView::new("rtbit_status")
        .title("Torrents")
        .item("all", IconName::Home, "All")
        .item("live", IconName::ArrowClockwise, "Live")
        .item("paused", IconName::Dismiss, "Paused")
        .item("error", IconName::ErrorCircle, "Error")
        .item("initializing", IconName::ArrowClockwise, "Initializing");
    // Read the selection after show. NavigationView writes the clicked item
    // during the frame; reading NavState before show always sees the filter
    // that was already active, so a click never changes it. The content
    // closure's value comes back as `inner`.
    let response = view.show(ui, &mut nav, |ui| {
        filters(ui, cache, category, tracker);
        if let Some(error) = &cache.error {
            ui.colored_label(egui::Color32::from_rgb(196, 43, 28), error);
            ui.add_space(4.0);
        }
        grid_panel(ui, cache, *status, category, tracker, grid)
    });
    if response.inner.is_some() {
        out.selected = response.inner;
    }
    if let Some(key) = response.selected.or(nav.selected.as_deref()) {
        *status = status_from_key(key);
    }

    if let Some(request) = add_dialog(ui.ctx(), rows.add_open, rows.draft) {
        out.add = Some(request);
    }

    if *rows.confirm_delete {
        let result = Dialog::new("rtbit_delete")
            .title("Delete torrent")
            .actions(|actions| {
                actions.primary("delete", "Delete");
                actions.secondary("cancel", "Cancel");
            })
            .show(ui.ctx(), rows.confirm_delete, |ui| {
                ui.label("Delete this torrent and its files?");
            });
        if let DialogResult::Closed(CloseReason::Button(id)) = result
            && id == "delete"
        {
            out.action = Some(RowAction::Delete);
        }
    }
    out
}

fn status_key(status: StatusFilter) -> &'static str {
    match status {
        StatusFilter::All => "all",
        StatusFilter::Live => "live",
        StatusFilter::Paused => "paused",
        StatusFilter::Error => "error",
        StatusFilter::Initializing => "initializing",
    }
}

fn status_from_key(key: &str) -> StatusFilter {
    match key {
        "live" => StatusFilter::Live,
        "paused" => StatusFilter::Paused,
        "error" => StatusFilter::Error,
        "initializing" => StatusFilter::Initializing,
        _ => StatusFilter::All,
    }
}

fn filters(ui: &mut egui::Ui, cache: &ListCache, category: &mut String, tracker: &mut String) {
    ui.add_space(8.0);
    ui.label("Status");
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

/// The add dialog. Returns a request only when Add is pressed and the
/// magnet (or file) and the save path pass the client checks.
fn add_dialog(ctx: &egui::Context, open: &mut bool, draft: &mut AddDraft) -> Option<AddRequest> {
    if !*open {
        return None;
    }
    let mut request = None;
    // The harness types and then presses Enter in the same frame, so the field
    // still holds the previous value when the dialog reads the key. Hold the
    // confirmation for one frame so validation sees the text that was typed.
    let confirm = ctx
        .data(|data| data.get_temp::<bool>(egui::Id::new("rtbit_add_confirm")))
        .unwrap_or(false);
    if confirm {
        ctx.data_mut(|data| data.remove::<bool>(egui::Id::new("rtbit_add_confirm")));
        request = match validate_add(draft) {
            Ok(built) => Some(built),
            Err(message) => {
                draft.error = message;
                *open = true;
                None
            }
        };
    }
    // "Add torrent", not "Add": the command bar already has an Add button, and a
    // click at this centred dialog's button lands on the bar behind it.
    let result = Dialog::new("rtbit_add")
        .title("Add torrent")
        .actions(|actions| {
            actions.primary("add", "Add torrent");
            actions.secondary("cancel", "Cancel");
        })
        .show(ctx, open, |ui| {
            ui.add(Input::new(&mut draft.magnet).label("Magnet link"));
            ui.add_space(8.0);
            let file_label = if draft.file_name.is_empty() {
                "No file chosen".to_owned()
            } else {
                draft.file_name.clone()
            };
            ui.horizontal(|ui| {
                if ui
                    .add(Button::new("Choose .torrent").appearance(Appearance::Secondary))
                    .clicked()
                    && let Some(picked) = pick_torrent_file()
                {
                    draft.file_name = picked.0;
                    draft.file_bytes = picked.1;
                }
                ui.label(file_label);
            });
            ui.add_space(8.0);
            ui.add(Input::new(&mut draft.save_path).label("Save path"));
            ui.add_space(8.0);
            ui.add(Input::new(&mut draft.category).label("Category"));
            ui.add_space(8.0);
            ui.add(Checkbox::bool(&mut draft.start_paused, "Start paused"));
            if !draft.error.is_empty() {
                ui.add_space(8.0);
                ui.colored_label(egui::Color32::from_rgb(196, 43, 28), &draft.error);
            }
        });
    match result {
        DialogResult::Closed(CloseReason::Button(id)) if id == "add" => {
            ctx.data_mut(|data| data.insert_temp(egui::Id::new("rtbit_add_confirm"), true));
            *open = true;
        }
        DialogResult::Closed(_) => {
            *draft = AddDraft::default();
        }
        DialogResult::Open(_) => {}
    }
    request
}

/// Client-side checks. The daemon still rejects a bad magnet or a bad path.
pub fn validate_add(draft: &AddDraft) -> Result<AddRequest, String> {
    let save_path = draft.save_path.trim().to_owned();
    if let Some(message) = check_save_path(&save_path) {
        return Err(message.to_owned());
    }
    let magnet = draft.magnet.trim();
    let source = if !magnet.is_empty() {
        if let Some(message) = check_magnet(magnet) {
            return Err(message.to_owned());
        }
        AddSource::Magnet(magnet.to_owned())
    } else if !draft.file_bytes.is_empty() {
        AddSource::File {
            name: draft.file_name.clone(),
            bytes: draft.file_bytes.clone(),
        }
    } else {
        return Err("enter a magnet link or choose a .torrent file".to_owned());
    };
    Ok(AddRequest {
        source,
        save_path,
        category: draft.category.trim().to_owned(),
        start_paused: draft.start_paused,
    })
}

fn check_magnet(magnet: &str) -> Option<&'static str> {
    let Some(rest) = magnet.strip_prefix("magnet:?") else {
        return Some("magnet link must start with magnet:?");
    };
    let ok = rest.split('&').any(|part| {
        let mut key_value = part.splitn(2, '=');
        key_value.next() == Some("xt")
            && key_value
                .next()
                .is_some_and(|value| value.to_ascii_lowercase().starts_with("urn:btih:"))
    });
    if magnet.starts_with("magnet:?") && ok {
        None
    } else if magnet.starts_with("magnet:?") {
        Some("magnet link needs xt=urn:btih:")
    } else {
        Some("magnet link must start with magnet:?")
    }
}

fn check_save_path(path: &str) -> Option<&'static str> {
    if path.is_empty() {
        return Some("save path is required");
    }
    if path.contains('\0') {
        return Some("save path contains a null");
    }
    if path.contains('\n') || path.contains('\r') {
        return Some("save path must be a single line");
    }
    None
}

/// Open the native file dialog when the `file-dialog` feature is on.
/// Tests leave it off, so a click records nothing and does not open a window.
fn pick_torrent_file() -> Option<(String, Vec<u8>)> {
    #[cfg(feature = "file-dialog")]
    {
        let path = rfd::FileDialog::new()
            .add_filter("Torrent", &["torrent"])
            .pick_file()?;
        let bytes = std::fs::read(&path).ok()?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("torrent")
            .to_owned();
        return Some((name, bytes));
    }
    #[cfg(not(feature = "file-dialog"))]
    {
        None
    }
}

#[cfg(test)]
mod add_checks {
    use super::*;

    #[test]
    fn a_magnet_without_a_hash_is_refused() {
        let draft = AddDraft {
            magnet: "magnet:?dn=nohash".into(),
            save_path: "/data".into(),
            ..AddDraft::default()
        };
        assert!(validate_add(&draft).is_err());
    }

    #[test]
    fn a_blank_path_is_refused() {
        let draft = AddDraft {
            magnet: "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567".into(),
            save_path: "  ".into(),
            ..AddDraft::default()
        };
        assert!(validate_add(&draft).is_err());
    }

    #[test]
    fn a_good_magnet_and_path_is_accepted() {
        let draft = AddDraft {
            magnet: "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Example"
                .into(),
            save_path: "/data/incoming".into(),
            category: "movies".into(),
            start_paused: true,
            ..AddDraft::default()
        };
        let request = validate_add(&draft).expect("valid");
        assert!(matches!(request.source, AddSource::Magnet(_)));
        assert_eq!(request.save_path, "/data/incoming");
        assert!(request.start_paused);
    }
}
