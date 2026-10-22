//! The mockup window: toolbar, sidebar, torrent table, detail pane and status bar, plus the
//! RSS, log and login screens and every dialog. All state is local; nothing talks to a server.

use std::collections::BTreeSet;
use std::time::Duration;

use egui::SetOpenCommand;
use egui::{Align, Layout, RichText, Ui};
use fluent2_egui::controls::{Appearance, Button, ButtonSize};
use fluent2_egui::office::status_bar::StatusBar;
use fluent2_egui::surfaces::{Intent, MessageBar, Toast, ToastPosition, Toaster};
use fluent2_egui::theme::{Density, Theme, ThemeKind, c};
use fluent2_egui::{
    Badge, BadgeColor, BadgeSize, Checkbox, CloseReason, DataGridSize, DataGridState, Dialog,
    DialogKind, DialogResult, Dropdown, Field, Icon, IconName, IconSize, Input, MenuButton,
    MenuItem, RowId, SearchBox, ToggleButton, TooltipExt, TypeRamp, divider, menu_frame, rich,
    submenu,
};

use crate::data::{self, CATEGORIES, Library, State};
use crate::detail::{self, DetailState};
use crate::log::{self as logview, LogState};
use crate::login::{self, LoginState};
use crate::rss::{self, RssState};
use crate::settings::{self, Settings, Stats};
use crate::table;

/// Top-level page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Screen {
    Login,
    #[default]
    Torrents,
    Rss,
    Logs,
}

/// A dialog to open at start, for screenshots and tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartDialog {
    Settings(usize),
    Magnet,
    AddFiles,
    Upload,
    Delete,
}

/// How the app starts: screen, theme, density, an open dialog. Parsed from native flags.
#[derive(Clone, Debug, Default)]
pub struct Launch {
    pub screen: Screen,
    pub dark: bool,
    pub compact: bool,
    pub dialog: Option<StartDialog>,
    pub detail_tab: usize,
}

impl Launch {
    /// `--screen login|torrents|rss|logs|settings|add|files|upload|delete`, `--dark`,
    /// `--compact`, `--tab N` (settings tab), `--detail N` (detail pane tab).
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Self {
        let mut launch = Self::default();
        let mut settings_tab = 0;
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--dark" => launch.dark = true,
                "--compact" => launch.compact = true,
                "--tab" => settings_tab = args.next().and_then(|v| v.parse().ok()).unwrap_or(0),
                "--detail" => {
                    launch.detail_tab = args.next().and_then(|v| v.parse().ok()).unwrap_or(0)
                }
                "--screen" => match args.next().as_deref() {
                    Some("login") => launch.screen = Screen::Login,
                    Some("rss") => launch.screen = Screen::Rss,
                    Some("logs" | "log") => launch.screen = Screen::Logs,
                    Some("settings") => launch.dialog = Some(StartDialog::Settings(0)),
                    Some("add" | "magnet") => launch.dialog = Some(StartDialog::Magnet),
                    Some("files") => launch.dialog = Some(StartDialog::AddFiles),
                    Some("upload") => launch.dialog = Some(StartDialog::Upload),
                    Some("delete") => launch.dialog = Some(StartDialog::Delete),
                    _ => launch.screen = Screen::Torrents,
                },
                _ => {}
            }
        }
        if let Some(StartDialog::Settings(tab)) = &mut launch.dialog {
            *tab = settings_tab;
        }
        launch
    }
}

/// Sidebar filter. `None` is "All".
#[derive(Clone, Debug, Default, PartialEq)]
struct Filters {
    status: Option<State>,
    /// `Some("")` is "Uncategorized".
    category: Option<&'static str>,
    /// `Some("")` is "Trackerless".
    tracker: Option<&'static str>,
}

/// The second step of adding: pick files and the output folder.
struct AddDraft {
    name: String,
    output: String,
    files: Vec<(String, u64, bool)>,
}

struct UploadDraft {
    output: String,
    category: Option<usize>,
    start: bool,
    files: Vec<(&'static str, Option<&'static str>)>,
    done: bool,
}

#[derive(Default)]
struct Dialogs {
    magnet: Option<String>,
    add: Option<AddDraft>,
    upload: Option<UploadDraft>,
    delete: Option<Vec<u64>>,
    delete_files: bool,
    settings: Option<Settings>,
    shortcuts: bool,
}

pub struct MockupApp {
    lib: Library,
    screen: Screen,
    dark: bool,
    density: Density,
    filters: Filters,
    search: String,
    grid: DataGridState,
    visible_columns: Vec<&'static str>,
    detail: DetailState,
    sidebar_collapsed: bool,
    alt_speed: bool,
    settings: Settings,
    dialogs: Dialogs,
    rss: RssState,
    log: LogState,
    login: LoginState,
    last_tick: f64,
    uptime_secs: u64,
    /// Rows visible this frame (indices into `lib.torrents`), filtered and sorted.
    rows: Vec<usize>,
    /// The theme is installed on the first frame, so tests can build the app without a context.
    theme_installed: bool,
}

impl MockupApp {
    pub fn new(ctx: &egui::Context, launch: Launch) -> Self {
        let mut app = Self::with_launch(launch);
        app.apply_theme(ctx);
        app.theme_installed = true;
        app
    }

    /// The app without a context; the theme is installed on the first frame.
    pub fn with_launch(launch: Launch) -> Self {
        let mut app = Self {
            lib: Library::generate(),
            screen: launch.screen,
            dark: launch.dark,
            density: if launch.compact {
                Density::Compact
            } else {
                Density::Medium
            },
            filters: Filters::default(),
            search: String::new(),
            grid: DataGridState {
                sort: Some(fluent2_egui::data::data_grid::SortSpec {
                    column: "id".to_owned(),
                    direction: fluent2_egui::data::data_grid::SortDirection::Descending,
                }),
                ..Default::default()
            },
            visible_columns: table::DEFAULT_VISIBLE.to_vec(),
            detail: DetailState {
                tab: launch.detail_tab,
                ..Default::default()
            },
            sidebar_collapsed: false,
            alt_speed: false,
            settings: Settings::default(),
            dialogs: Dialogs::default(),
            rss: RssState::default(),
            log: LogState::default(),
            login: LoginState {
                username: "admin".to_owned(),
                ..Default::default()
            },
            last_tick: 0.0,
            uptime_secs: 3 * 86_400 + 4 * 3600 + 12 * 60,
            rows: Vec::new(),
            theme_installed: false,
        };
        // Start with a downloading torrent selected, so the detail pane has content.
        if let Some(t) = app
            .lib
            .torrents
            .iter()
            .rev()
            .find(|t| t.state == State::Downloading && t.peer_list.len() > 4)
        {
            app.grid.selection.selected.insert(RowId(t.id));
            app.grid.selection.anchor = Some(RowId(t.id));
        }
        match launch.dialog {
            Some(StartDialog::Settings(tab)) => {
                app.dialogs.settings = Some(Settings {
                    tab,
                    ..app.settings.clone()
                });
            }
            Some(StartDialog::Magnet) => app.dialogs.magnet = Some(String::new()),
            Some(StartDialog::AddFiles) => {
                app.open_add("magnet:?xt=urn:btih:cafe&dn=Sintel.2010.2160p.WEB-DL.HEVC")
            }
            Some(StartDialog::Upload) => app.open_upload(),
            Some(StartDialog::Delete) => {
                let ids: Vec<u64> = app
                    .lib
                    .torrents
                    .iter()
                    .rev()
                    .take(3)
                    .map(|t| t.id)
                    .collect();
                app.dialogs.delete = Some(ids);
            }
            None => {}
        }
        app
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        let brand = Theme::default().brand;
        let theme = if self.dark {
            Theme::dark(brand)
        } else {
            Theme::light(brand)
        }
        .with_density(self.density);
        fluent2_egui::install(ctx, &theme);
    }

    fn grid_size(&self) -> DataGridSize {
        match self.density {
            Density::Compact => DataGridSize::ExtraSmall,
            Density::Touch => DataGridSize::Medium,
            _ => DataGridSize::Small,
        }
    }

    fn selected_ids(&self) -> Vec<u64> {
        self.grid.selection.selected.iter().map(|r| r.0).collect()
    }

    fn toast(ctx: &egui::Context, intent: Intent, title: impl Into<String>, body: Option<String>) {
        Toaster::push(
            ctx,
            Toast {
                intent,
                title: title.into(),
                body,
                timeout: Some(Duration::from_secs(4)),
                action: None,
            },
        );
    }

    fn open_add(&mut self, link: &str) {
        let name = link
            .split("dn=")
            .nth(1)
            .map(|s| {
                s.split('&')
                    .next()
                    .unwrap_or(s)
                    .replace("%20", " ")
                    .replace('+', " ")
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Pioneer.One.Complete.1080p.WEB.x265".to_owned());
        let files = vec![
            (format!("{name}/{name}.mkv"), 4_200_000_000, true),
            (format!("{name}/Subs/English.srt"), 81_000, true),
            (format!("{name}/Subs/German.srt"), 79_000, true),
            (format!("{name}/sample.mkv"), 64_000_000, false),
            (format!("{name}/{name}.nfo"), 3_000, true),
        ];
        self.dialogs.add = Some(AddDraft {
            name,
            output: String::new(),
            files,
        });
    }

    fn open_upload(&mut self) {
        self.dialogs.upload = Some(UploadDraft {
            output: String::new(),
            category: None,
            start: true,
            files: vec![
                ("debian-13.1.0-amd64-netinst.iso.torrent", None),
                ("Sintel.2010.2160p.WEB-DL.HEVC.torrent", None),
                ("Kevin MacLeod - Royalty Free Collection.torrent", None),
                (
                    "corrupt-download.torrent",
                    Some("invalid bencode at byte 412"),
                ),
            ],
            done: false,
        });
    }

    /// Draw one frame. `eframe::App::ui` calls this; so do the kittest smoke tests.
    pub fn show(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if !self.theme_installed {
            self.apply_theme(&ctx);
            self.theme_installed = true;
            // Fonts registered this frame are bound from the next one; draw from then on.
            ctx.request_repaint();
            return;
        }
        let now = ctx.input(|i| i.time);
        if now - self.last_tick >= 1.0 {
            self.last_tick = now;
            self.lib.tick();
            self.uptime_secs += 1;
            let names: Vec<&str> = self
                .lib
                .torrents
                .iter()
                .take(12)
                .map(|t| t.name.as_str())
                .collect();
            self.log.tick(&names);
        }
        ctx.request_repaint_after(Duration::from_millis(1000));

        if self.screen == Screen::Login {
            egui::CentralPanel::default().show(ui, |ui| {
                if login::show(ui, &mut self.login) {
                    self.screen = Screen::Torrents;
                    Self::toast(
                        &ctx,
                        Intent::Success,
                        "Signed in",
                        Some(format!("Welcome back, {}.", self.login.username)),
                    );
                }
                ui.with_layout(Layout::bottom_up(Align::Max), |ui| self.theme_toggle(ui));
            });
            Toaster::show(&ctx, ToastPosition::BottomEnd);
            return;
        }

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status")
            .show_separator_line(false)
            .show(ui, |ui| self.status_bar(ui));
        match self.screen {
            Screen::Torrents => self.torrents_screen(ui),
            Screen::Rss => {
                let size = self.grid_size();
                egui::CentralPanel::default().show(ui, |ui| {
                    if let Some(msg) = rss::show(ui, &mut self.rss, size) {
                        Self::toast(&ctx, Intent::Success, msg, None);
                    }
                });
            }
            _ => {
                egui::CentralPanel::default().show(ui, |ui| logview::show(ui, &mut self.log));
            }
        }
        self.dialogs(&ctx);
        Toaster::show(&ctx, ToastPosition::BottomEnd);
        self.shortcuts(&ctx);
    }

    fn theme_toggle(&mut self, ui: &mut Ui) {
        let (icon, label) = if self.dark {
            (IconName::WeatherSunny, "Light theme")
        } else {
            (IconName::WeatherMoon, "Dark theme")
        };
        if ui
            .add(
                Button::icon_only(icon, label)
                    .appearance(Appearance::Subtle)
                    .id_salt("theme"),
            )
            .fluent_tooltip(label)
            .clicked()
        {
            self.dark = !self.dark;
            self.apply_theme(ui.ctx());
            ui.set_style(ui.ctx().global_style());
        }
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add(
                Icon::new(IconName::ArrowDownload)
                    .filled()
                    .size(IconSize::S24)
                    .tint(c(Theme::get(ui.ctx()).color.brand_foreground_1)),
            );
            ui.label(rich("rustTorrent", TypeRamp::Subtitle2));
            ui.separator();
            for (screen, label) in [
                (Screen::Torrents, "Torrents"),
                (Screen::Rss, "RSS"),
                (Screen::Logs, "Logs"),
            ] {
                if ui.selectable_label(self.screen == screen, label).clicked() {
                    self.screen = screen;
                }
            }
            if self.screen == Screen::Torrents {
                ui.add_space(12.0);
                ui.add(
                    SearchBox::new(&mut self.search)
                        .placeholder("Filter torrents…")
                        .desired_width(200.0)
                        .id_salt("search"),
                );
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(
                        Button::icon_only(IconName::ArrowLeft, "Log out")
                            .appearance(Appearance::Subtle)
                            .id_salt("logout"),
                    )
                    .fluent_tooltip("Log out")
                    .clicked()
                {
                    self.screen = Screen::Login;
                    self.login.password.clear();
                }
                self.theme_toggle(ui);
                self.density_menu(ui);
                if ui
                    .add(
                        Button::icon_only(IconName::Document, "View logs")
                            .appearance(Appearance::Subtle)
                            .id_salt("logs"),
                    )
                    .fluent_tooltip("View logs (Ctrl+L)")
                    .clicked()
                {
                    self.screen = Screen::Logs;
                }
                if ui
                    .add(
                        Button::icon_only(IconName::Settings, "Settings")
                            .appearance(Appearance::Subtle)
                            .id_salt("settings"),
                    )
                    .fluent_tooltip("Settings (Ctrl+,)")
                    .clicked()
                {
                    self.dialogs.settings = Some(self.settings.clone());
                }
                ui.separator();
                if self.screen == Screen::Torrents {
                    self.actions(ui);
                }
            });
        });
        ui.add_space(4.0);
    }

    fn density_menu(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        MenuButton::new("View")
            .icon(IconName::Options)
            .appearance(Appearance::Subtle)
            .id_salt("view")
            .show(ui, |ui| {
                fluent2_egui::group_header(ui, "Density");
                for (label, d) in [
                    ("Compact", Density::Compact),
                    ("Medium", Density::Medium),
                    ("Touch", Density::Touch),
                ] {
                    if ui
                        .add(MenuItem::new(label).checked(self.density == d))
                        .clicked()
                    {
                        self.density = d;
                        self.apply_theme(&ctx);
                    }
                }
                divider(ui);
                fluent2_egui::group_header(ui, "Columns");
                for (id, header) in table::COLUMNS {
                    if id == "name" {
                        continue;
                    }
                    let on = self.visible_columns.contains(&id);
                    let label = if header.is_empty() {
                        "Status icon"
                    } else {
                        header
                    };
                    if ui
                        .add(MenuItem::new(label).checked(on).id_salt(("col", id)))
                        .clicked()
                    {
                        if on {
                            self.visible_columns.retain(|c| *c != id);
                        } else {
                            // Keep display order.
                            self.visible_columns.push(id);
                            let order: Vec<&str> = table::COLUMNS.iter().map(|(i, _)| *i).collect();
                            self.visible_columns
                                .sort_by_key(|c| order.iter().position(|o| o == c));
                        }
                    }
                }
                if ui.add(MenuItem::new("Reset to defaults")).clicked() {
                    self.visible_columns = table::DEFAULT_VISIBLE.to_vec();
                }
                divider(ui);
                if ui
                    .add(
                        MenuItem::new("Show sidebar")
                            .checked(!self.sidebar_collapsed)
                            .shortcut("B"),
                    )
                    .clicked()
                {
                    self.sidebar_collapsed = !self.sidebar_collapsed;
                }
                if ui
                    .add(MenuItem::new("Keyboard shortcuts").shortcut("?"))
                    .clicked()
                {
                    self.dialogs.shortcuts = true;
                }
            });
    }

    /// Add, resume, pause and delete, with a selected-count badge. Laid out right to left.
    fn actions(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let ids = self.selected_ids();
        let any = !ids.is_empty();
        if ui
            .add_enabled(
                any,
                Button::icon_only(IconName::Delete, "Delete selected")
                    .appearance(Appearance::Subtle)
                    .id_salt("del"),
            )
            .fluent_tooltip("Delete selected (Delete)")
            .clicked()
        {
            self.dialogs.delete = Some(ids.clone());
        }
        if ui
            .add_enabled(
                any,
                Button::icon_only(IconName::Subtract, "Pause selected")
                    .appearance(Appearance::Subtle)
                    .id_salt("pause"),
            )
            .fluent_tooltip("Pause selected (Space)")
            .clicked()
        {
            self.lib.set_state(&ids, State::Paused);
            Self::toast(
                &ctx,
                Intent::Info,
                format!("Paused {} torrent(s)", ids.len()),
                None,
            );
        }
        if ui
            .add_enabled(
                any,
                Button::icon_only(IconName::ArrowRight, "Resume selected")
                    .appearance(Appearance::Subtle)
                    .id_salt("resume"),
            )
            .fluent_tooltip("Resume selected (Space)")
            .clicked()
        {
            self.lib.set_state(&ids, State::Downloading);
            Self::toast(
                &ctx,
                Intent::Success,
                format!("Resumed {} torrent(s)", ids.len()),
                None,
            );
        }
        if any {
            ui.add(
                Badge::new(format!("{} selected", ids.len()))
                    .color(BadgeColor::Brand)
                    .size(BadgeSize::Small),
            );
        }
        if ui
            .add(
                Button::new("Upload")
                    .icon(IconName::Document)
                    .id_salt("upload"),
            )
            .clicked()
        {
            self.open_upload();
        }
        if ui
            .add(
                Button::new("Add magnet")
                    .icon(IconName::Link)
                    .appearance(Appearance::Primary)
                    .id_salt("magnet"),
            )
            .clicked()
        {
            self.dialogs.magnet = Some(String::new());
        }
    }

    fn matches(&self, t: &data::Torrent, f: &Filters) -> bool {
        let status = match f.status {
            None => true,
            Some(State::Downloading) => matches!(
                t.state,
                State::Downloading | State::Stalled | State::Checking
            ),
            Some(s) => t.state == s,
        };
        status
            && f.category.is_none_or(|c| t.category == c)
            && f.tracker.is_none_or(|h| t.tracker_host == h)
            && (self.search.is_empty()
                || t.name.to_lowercase().contains(&self.search.to_lowercase()))
    }

    fn torrents_screen(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if !self.sidebar_collapsed {
            egui::Panel::left("sidebar")
                .resizable(false)
                .exact_size(208.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.sidebar(ui));
                });
        }
        egui::Panel::bottom("detail")
            .resizable(true)
            .default_size(280.0)
            .min_size(120.0)
            .max_size(620.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                let size = self.grid_size();
                let selected: Vec<&data::Torrent> = self
                    .grid
                    .selection
                    .selected
                    .iter()
                    .filter_map(|id| self.lib.by_id(id.0))
                    .collect();
                detail::show(ui, &mut self.detail, &selected, size);
            });

        let filters = self.filters.clone();
        let mut rows: Vec<usize> = (0..self.lib.torrents.len())
            .filter(|&i| self.matches(&self.lib.torrents[i], &filters))
            .collect();
        table::sort(&mut rows, &self.lib.torrents, &self.grid);
        self.rows = rows;

        let size = self.grid_size();
        egui::CentralPanel::default().show(ui, |ui| {
            if self.lib.torrents.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label("No torrents yet\nPaste a magnet link or drop a .torrent file anywhere on this page to add one.")
                });
                return;
            }
            if self.rows.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(rich("No matches", TypeRamp::Subtitle2));
                    ui.label("No torrents match the current search or filters.");
                    if ui.add(Button::new("Clear filters")).clicked() {
                        self.filters = Filters::default();
                        self.search.clear();
                    }
                });
                return;
            }
            let out = table::show(ui, &self.lib.torrents, &self.rows, &self.visible_columns, &mut self.grid, size);
            let right_clicked = out.response.context_menu;
            if let Some(cell) = right_clicked
                && !self.grid.selection.selected.contains(&cell.row_id)
            {
                self.grid.selection.selected = BTreeSet::from([cell.row_id]);
                self.grid.selection.anchor = Some(cell.row_id);
            }
            if out.response.activated.is_some() {
                self.detail.tab = 0;
            }
            // The grid takes the secondary click itself and reports it, so the menu opens from
            // that report rather than from the grid response's own click state.
            let frame = menu_frame(ui);
            egui::Popup::menu(&out.response.response)
                .open_memory(right_clicked.map(|_| SetOpenCommand::Bool(true)))
                .at_pointer_fixed()
                .frame(frame)
                .show(|ui| self.row_menu(ui, &ctx));
        });
    }

    fn sidebar_item(
        ui: &mut Ui,
        selected: bool,
        icon: IconName,
        label: &str,
        count: usize,
    ) -> bool {
        let mut clicked = false;
        ui.horizontal(|ui| {
            ui.add(Icon::new(icon).size(IconSize::S16));
            let r = ui.selectable_label(selected, label);
            clicked = r.clicked();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(count.to_string()).weak());
            });
        });
        clicked
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        let torrents = &self.lib.torrents;
        let count = |f: &dyn Fn(&data::Torrent) -> bool| torrents.iter().filter(|t| f(t)).count();
        ui.add_space(6.0);
        ui.label(rich("STATUS", TypeRamp::Caption1).strong());
        let statuses: [(Option<State>, IconName, &str); 6] = [
            (None, IconName::Grid, "All"),
            (
                Some(State::Downloading),
                IconName::ArrowDownload,
                "Downloading",
            ),
            (Some(State::Seeding), IconName::ArrowUp, "Seeding"),
            (Some(State::Paused), IconName::Subtract, "Paused"),
            (Some(State::Queued), IconName::MoreHorizontal, "Queued"),
            (Some(State::Error), IconName::ErrorCircle, "Error"),
        ];
        for (state, icon, label) in statuses {
            let n = match state {
                None => torrents.len(),
                Some(State::Downloading) => count(&|t| {
                    matches!(
                        t.state,
                        State::Downloading | State::Stalled | State::Checking
                    )
                }),
                Some(s) => count(&|t| t.state == s),
            };
            if Self::sidebar_item(ui, self.filters.status == state, icon, label, n) {
                self.filters.status = state;
            }
        }
        ui.add_space(10.0);
        ui.label(rich("CATEGORIES", TypeRamp::Caption1).strong());
        if Self::sidebar_item(
            ui,
            self.filters.category.is_none(),
            IconName::Grid,
            "All",
            torrents.len(),
        ) {
            self.filters.category = None;
        }
        let n = count(&|t| t.category.is_empty());
        if Self::sidebar_item(
            ui,
            self.filters.category == Some(""),
            IconName::Bookmark,
            "Uncategorized",
            n,
        ) {
            self.filters.category = Some("");
        }
        let mut cats = CATEGORIES;
        cats.sort();
        for cat in cats {
            let n = count(&|t| t.category == cat);
            if Self::sidebar_item(
                ui,
                self.filters.category == Some(cat),
                IconName::Bookmark,
                cat,
                n,
            ) {
                self.filters.category = Some(cat);
            }
        }
        ui.add_space(10.0);
        ui.label(rich("TRACKERS", TypeRamp::Caption1).strong());
        if Self::sidebar_item(
            ui,
            self.filters.tracker.is_none(),
            IconName::Grid,
            "All",
            torrents.len(),
        ) {
            self.filters.tracker = None;
        }
        let n = count(&|t| t.tracker_host.is_empty());
        if n > 0
            && Self::sidebar_item(
                ui,
                self.filters.tracker == Some(""),
                IconName::Share,
                "Trackerless",
                n,
            )
        {
            self.filters.tracker = Some("");
        }
        let mut hosts = data::TRACKERS;
        hosts.sort();
        for host in hosts {
            let n = count(&|t| t.tracker_host == host);
            if Self::sidebar_item(
                ui,
                self.filters.tracker == Some(host),
                IconName::Share,
                host,
                n,
            ) {
                self.filters.tracker = Some(host);
            }
        }
    }

    fn row_menu(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        ui.set_min_width(240.0);
        let ids = self.selected_ids();
        let n = ids.len();
        let plural = |one: &str, many: &str| {
            if n > 1 {
                many.to_owned()
            } else {
                one.to_owned()
            }
        };
        let targets: Vec<data::Torrent> = ids
            .iter()
            .filter_map(|id| self.lib.by_id(*id).cloned())
            .collect();
        let can_resume = targets.iter().any(|t| !t.state.is_active());
        let can_pause = targets.iter().any(|t| t.state.is_active());
        let any_finished = targets.iter().any(|t| t.progress >= 1.0);
        let any_queued = targets.iter().any(|t| t.queue_position.is_some());
        let sequential = targets.iter().all(|t| t.sequential);
        let super_seed = targets.iter().all(|t| t.super_seed);
        let first_name = targets.first().map(|t| t.name.clone()).unwrap_or_default();
        let first_hash = targets
            .first()
            .map(|t| t.info_hash.clone())
            .unwrap_or_default();

        if can_resume
            && ui
                .add(
                    MenuItem::new(plural("Resume", "Resume Selected"))
                        .icon(IconName::ArrowRight)
                        .shortcut("Space"),
                )
                .clicked()
        {
            self.lib.set_state(&ids, State::Downloading);
            Self::toast(
                ctx,
                Intent::Success,
                format!("Resumed {n} torrent(s)"),
                None,
            );
        }
        if can_pause
            && ui
                .add(
                    MenuItem::new(plural("Pause", "Pause Selected"))
                        .icon(IconName::Subtract)
                        .shortcut("Space"),
                )
                .clicked()
        {
            self.lib.set_state(&ids, State::Paused);
            Self::toast(ctx, Intent::Info, format!("Paused {n} torrent(s)"), None);
        }
        divider(ui);
        if ui
            .add(
                MenuItem::new(plural("Force Recheck", "Force Recheck Selected"))
                    .icon(IconName::ArrowClockwise),
            )
            .clicked()
        {
            self.lib.set_state(&ids, State::Checking);
            Self::toast(
                ctx,
                Intent::Info,
                "Rechecking pieces",
                Some(first_name.clone()),
            );
        }
        divider(ui);
        if n == 1
            && ui
                .add(MenuItem::new("Configure Files").icon(IconName::Folder))
                .clicked()
        {
            self.detail.tab = 1;
        }
        if ui
            .add(MenuItem::new("Sequential Download").checked(sequential))
            .clicked()
        {
            for id in &ids {
                if let Some(t) = self.lib.by_id_mut(*id) {
                    t.sequential = !sequential;
                }
            }
        }
        if any_finished
            && ui
                .add(MenuItem::new("Super-Seed").checked(super_seed))
                .clicked()
        {
            for id in &ids {
                if let Some(t) = self.lib.by_id_mut(*id) {
                    t.super_seed = !super_seed;
                }
            }
        }
        divider(ui);
        if any_queued {
            for label in ["Move to Top", "Move Up", "Move Down", "Move to Bottom"] {
                if ui.add(MenuItem::new(label)).clicked() {
                    Self::toast(ctx, Intent::Info, format!("{label}: {n} torrent(s)"), None);
                }
            }
            divider(ui);
        }
        if n == 1 {
            if ui
                .add(MenuItem::new("Set Speed Limits...").icon(IconName::ArrowSort))
                .clicked()
            {
                Self::toast(
                    ctx,
                    Intent::Info,
                    "Speed limits",
                    Some("Per-torrent limits are not wired in the mockup.".into()),
                );
            }
            if ui
                .add(MenuItem::new("Set Seed Limits...").icon(IconName::ArrowUp))
                .clicked()
            {
                Self::toast(
                    ctx,
                    Intent::Info,
                    "Seed limits",
                    Some("Per-torrent limits are not wired in the mockup.".into()),
                );
            }
            divider(ui);
        }
        submenu(ui, "Set Category...", Some(IconName::Bookmark), |ui| {
            let current = targets.first().map(|t| t.category).unwrap_or("");
            let mut pick = None;
            if ui
                .add(MenuItem::new("None").checked(current.is_empty()))
                .clicked()
            {
                pick = Some("");
            }
            for cat in CATEGORIES {
                if ui.add(MenuItem::new(cat).checked(current == cat)).clicked() {
                    pick = Some(cat);
                }
            }
            divider(ui);
            if ui.add(MenuItem::new("+ New Category...")).clicked() {
                Self::toast(
                    ctx,
                    Intent::Info,
                    "New category",
                    Some("Category editing is not wired in the mockup.".into()),
                );
            }
            if let Some(cat) = pick {
                for id in &ids {
                    if let Some(t) = self.lib.by_id_mut(*id) {
                        t.category = cat;
                    }
                }
                Self::toast(
                    ctx,
                    Intent::Success,
                    format!(
                        "Category set to {}",
                        if cat.is_empty() { "None" } else { cat }
                    ),
                    None,
                );
            }
        });
        divider(ui);
        for (label, value) in [
            ("Copy Name", first_name.clone()),
            ("Copy Info Hash", first_hash.clone()),
            (
                "Copy Magnet Link",
                format!("magnet:?xt=urn:btih:{first_hash}"),
            ),
        ] {
            if ui.add(MenuItem::new(label).icon(IconName::Copy)).clicked() {
                ui.ctx().copy_text(value);
                Self::toast(ctx, Intent::Success, "Copied to clipboard", None);
            }
        }
        divider(ui);
        let label = if n > 1 {
            format!("Delete {n} Torrents...")
        } else {
            "Delete...".to_owned()
        };
        let red = c(Theme::get(ui.ctx()).color.status_danger_foreground_1);
        if ui
            .add(
                MenuItem::new(RichText::new(label).color(red))
                    .icon(IconName::Delete)
                    .shortcut("Delete"),
            )
            .clicked()
        {
            self.dialogs.delete = Some(ids.clone());
        }
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        let (down, up) = (self.lib.total_down(), self.lib.total_up());
        let peers: u32 = self.lib.torrents.iter().map(|t| t.peers + t.seeds).sum();
        let total_down: u64 = self.lib.torrents.iter().map(|t| t.downloaded()).sum();
        let total_up: u64 = self.lib.torrents.iter().map(|t| t.uploaded).sum();
        let uptime = data::eta(Some(self.uptime_secs));
        let theme = Theme::get(ui.ctx());
        let alt = &mut self.alt_speed;
        let ctx = ui.ctx().clone();
        let spark: Vec<f32> = self
            .lib
            .global_down
            .iter()
            .rev()
            .take(40)
            .rev()
            .copied()
            .collect();
        StatusBar::new("status")
            .left(|ui| {
                let tip = if *alt {
                    "Alternative speed limits are ON — click to turn off"
                } else {
                    "Alternative speed limits are OFF — click to turn on"
                };
                if ui
                    .add(ToggleButton::new(alt, "Alt speed").size(ButtonSize::Small))
                    .fluent_tooltip(tip)
                    .changed()
                {
                    let msg = if *alt {
                        "Alternative speed limits on"
                    } else {
                        "Alternative speed limits off"
                    };
                    Self::toast(&ctx, Intent::Info, msg, None);
                }
                ui.separator();
                ui.label(
                    RichText::new(format!("↓ {}", nz(data::speed(down))))
                        .color(c(theme.color.brand_foreground_1))
                        .strong(),
                );
                ui.label(RichText::new(format!("({})", data::bytes(total_down))).weak());
                ui.add(
                    fluent2_egui::controls::Sparkline::new(&spark)
                        .size(egui::vec2(80.0, 16.0))
                        .label("Download speed"),
                );
                ui.label(
                    RichText::new(format!("↑ {}", nz(data::speed(up))))
                        .color(c(theme.color.status_success_foreground_1))
                        .strong(),
                );
                ui.label(RichText::new(format!("({})", data::bytes(total_up))).weak());
            })
            .right(|ui| {
                // The right slot lays out from the right edge, so items go in reverse.
                ui.hyperlink_to("API", "http://127.0.0.1:3030/swagger/");
                ui.separator();
                ui.label(format!("Up {uptime}"));
                ui.add(Icon::new(IconName::Calendar).size(IconSize::S16));
                ui.separator();
                ui.label(format!("{peers} peers"));
                ui.add(Icon::new(IconName::Person).size(IconSize::S16));
                ui.separator();
                ui.label(format!("{} torrents", self.lib.torrents.len()));
            })
            .show(ui);
    }

    fn stats(&self) -> Stats {
        Stats {
            uptime: data::eta(Some(self.uptime_secs)),
            downloaded: data::bytes(self.lib.torrents.iter().map(|t| t.downloaded()).sum()),
            uploaded: data::bytes(self.lib.torrents.iter().map(|t| t.uploaded).sum()),
            live_peers: self.lib.torrents.iter().map(|t| t.peers + t.seeds).sum(),
            down: nz(data::speed(self.lib.total_down())),
            up: nz(data::speed(self.lib.total_up())),
            feeds: self.rss.feeds.len(),
            rules: self.rss.rules.len(),
            items: self.rss.items.len(),
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        // Settings.
        let stats = self.stats();
        if let Some(draft) = &mut self.dialogs.settings {
            let mut open = true;
            let result = Dialog::new("settings")
                .title("Settings")
                .width(760.0)
                .actions(|a| {
                    a.primary("OK");
                    a.secondary("Cancel");
                })
                .show(ctx, &mut open, |ui| settings::body(ui, draft, &stats));
            if let DialogResult::Closed(reason) = result {
                if reason == CloseReason::Button("OK") {
                    self.settings = draft.clone();
                    Self::toast(ctx, Intent::Success, "Settings saved", None);
                }
                self.dialogs.settings = None;
            }
        }

        // Step 1: magnet or URL.
        if let Some(link) = &mut self.dialogs.magnet {
            let mut open = true;
            let result = Dialog::new("magnet")
                .title("Add torrent")
                .width(520.0)
                .actions(|a| {
                    a.primary("Add");
                    a.secondary("Cancel");
                })
                .show(ctx, &mut open, |ui| {
                    Field::new("Magnet link or URL")
                        .hint("Enter magnet or HTTP(S) URL to the .torrent")
                        .show(ui, |ui| {
                            ui.add(
                                Input::new(link)
                                    .placeholder("magnet:?xt=urn:btih:...")
                                    .desired_width(480.0)
                                    .id_salt("magnet_in"),
                            )
                        });
                });
            if let DialogResult::Closed(reason) = result {
                let link = self.dialogs.magnet.take().unwrap_or_default();
                if reason == CloseReason::Button("Add") {
                    self.open_add(&link);
                }
            }
        }

        // Step 2: choose files.
        if let Some(draft) = &mut self.dialogs.add {
            let mut open = true;
            let any = draft.files.iter().any(|f| f.2);
            let result = Dialog::new("add_files")
                .title("Add Torrent")
                .width(620.0)
                .actions(|a| {
                    a.primary("OK");
                    a.secondary("Cancel");
                })
                .show(ctx, &mut open, |ui| {
                    ui.label(rich(&draft.name, TypeRamp::Body1Strong));
                    ui.add_space(6.0);
                    Field::new("Output folder").show(ui, |ui| {
                        ui.add(
                            Input::new(&mut draft.output)
                                .placeholder("Server default")
                                .desired_width(580.0)
                                .id_salt("out"),
                        )
                    });
                    ui.add_space(6.0);
                    let total: u64 = draft.files.iter().filter(|f| f.2).map(|f| f.1).sum();
                    ui.label(format!("Files · {} selected", data::bytes(total)));
                    for (i, (name, size, on)) in draft.files.iter_mut().enumerate() {
                        ui.push_id(i, |ui| {
                            ui.horizontal(|ui| {
                                ui.add(Checkbox::bool(on, name.as_str()));
                                ui.label(RichText::new(data::bytes(*size)).weak());
                            });
                        });
                    }
                    if !any {
                        MessageBar::new(Intent::Warning, "Select at least one file.").show(ui);
                    }
                });
            if let DialogResult::Closed(reason) = result {
                let draft = self.dialogs.add.take();
                if reason == CloseReason::Button("OK")
                    && let Some(d) = draft
                {
                    if any {
                        self.lib.add(&d.name, "", false);
                        Self::toast(ctx, Intent::Success, "Torrent added", Some(d.name));
                    } else {
                        Self::toast(
                            ctx,
                            Intent::Warning,
                            "Nothing added",
                            Some("No files were selected.".into()),
                        );
                    }
                }
            }
        }

        // Upload multiple .torrent files.
        if let Some(draft) = &mut self.dialogs.upload {
            let mut open = true;
            let ok = draft.files.iter().filter(|f| f.1.is_none()).count();
            let primary: &'static str = if draft.done { "Close" } else { "Upload All" };
            let result = Dialog::new("upload")
                .title("Upload Multiple Torrents")
                .width(560.0)
                .actions(|a| {
                    a.primary(primary);
                    if !draft.done {
                        a.secondary("Cancel");
                    }
                })
                .show(ctx, &mut open, |ui| {
                    Field::new("Output folder").show(ui, |ui| {
                        ui.add(
                            Input::new(&mut draft.output)
                                .placeholder("Server default")
                                .desired_width(520.0)
                                .id_salt("up_out"),
                        )
                    });
                    Field::new("Category").show(ui, |ui| {
                        let mut names = vec!["None"];
                        names.extend(CATEGORIES);
                        ui.add(Dropdown::new("up_cat", &mut draft.category, &names).width(220.0));
                    });
                    ui.add(Checkbox::bool(
                        &mut draft.start,
                        "Start torrents after adding",
                    ));
                    ui.add_space(6.0);
                    ui.label(format!("{} files · {ok} ready", draft.files.len()));
                    for (name, error) in &draft.files {
                        ui.horizontal(|ui| {
                            let theme = Theme::get(ui.ctx());
                            let (icon, tint) = match (error, draft.done) {
                                (Some(_), _) => (
                                    IconName::ErrorCircle,
                                    c(theme.color.status_danger_foreground_1),
                                ),
                                (None, true) => (
                                    IconName::Checkmark,
                                    c(theme.color.status_success_foreground_1),
                                ),
                                (None, false) => {
                                    (IconName::Document, c(theme.color.neutral_foreground_3))
                                }
                            };
                            ui.add(Icon::new(icon).size(IconSize::S16).tint(tint));
                            ui.label(*name);
                            if let Some(e) = error {
                                ui.label(RichText::new(*e).small().color(tint));
                            }
                        });
                    }
                });
            if let DialogResult::Closed(reason) = result {
                if reason == CloseReason::Button("Upload All") {
                    let category = draft
                        .category
                        .and_then(|i| i.checked_sub(1))
                        .map(|i| CATEGORIES[i])
                        .unwrap_or("");
                    let start = draft.start;
                    let names: Vec<&str> = draft
                        .files
                        .iter()
                        .filter(|f| f.1.is_none())
                        .map(|f| f.0.trim_end_matches(".torrent"))
                        .collect();
                    for name in &names {
                        self.lib.add(name, category, !start);
                    }
                    if let Some(d) = &mut self.dialogs.upload {
                        d.done = true;
                    }
                    Self::toast(
                        ctx,
                        Intent::Success,
                        format!("Uploaded {} torrents", names.len()),
                        Some("1 file failed to parse.".into()),
                    );
                } else {
                    self.dialogs.upload = None;
                }
            }
        }

        // Delete confirmation.
        if let Some(ids) = &self.dialogs.delete {
            let ids = ids.clone();
            let mut open = true;
            let n = ids.len();
            let names: Vec<String> = ids
                .iter()
                .map(|id| {
                    self.lib
                        .by_id(*id)
                        .map(|t| t.name.clone())
                        .unwrap_or_else(|| format!("Torrent #{id}"))
                })
                .collect();
            let primary: &'static str = if n == 1 {
                "Delete Torrent"
            } else {
                "Delete Torrents"
            };
            let delete_files = &mut self.dialogs.delete_files;
            let result = Dialog::new("delete")
                .title(if n == 1 {
                    "Delete torrent".to_owned()
                } else {
                    format!("Delete {n} torrents")
                })
                .kind(DialogKind::Alert)
                .width(520.0)
                .actions(|a| {
                    a.primary(primary);
                    a.secondary("Cancel");
                })
                .show(ctx, &mut open, |ui| {
                    ui.label(if n == 1 {
                        "Are you sure you want to delete this torrent?"
                    } else {
                        "Are you sure you want to delete the following torrents?"
                    });
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height(160.0)
                        .show(ui, |ui| {
                            for name in &names {
                                ui.label(RichText::new(format!("• {name}")).strong());
                            }
                        });
                    ui.add_space(6.0);
                    ui.add(Checkbox::bool(delete_files, "Also delete downloaded files"));
                    if *delete_files {
                        MessageBar::new(
                            Intent::Warning,
                            "Downloaded data will be removed from disk.",
                        )
                        .show(ui);
                    }
                });
            if let DialogResult::Closed(reason) = result {
                if reason == CloseReason::Button(primary) {
                    self.lib.remove(&ids);
                    for id in &ids {
                        self.grid.selection.selected.remove(&RowId(*id));
                    }
                    let body = if self.dialogs.delete_files {
                        "Files were deleted too."
                    } else {
                        "Files were kept on disk."
                    };
                    Self::toast(
                        ctx,
                        Intent::Success,
                        format!("Deleted {n} torrent(s)"),
                        Some(body.into()),
                    );
                }
                self.dialogs.delete = None;
                self.dialogs.delete_files = false;
            }
        }

        if self.dialogs.shortcuts {
            let mut open = true;
            let result = Dialog::new("shortcuts")
                .title("Keyboard shortcuts")
                .kind(DialogKind::NonModal)
                .actions(|a| a.primary("Close"))
                .show(ctx, &mut open, |ui| {
                    egui::Grid::new("keys")
                        .num_columns(2)
                        .spacing([24.0, 6.0])
                        .show(ui, |ui| {
                            for (k, v) in [
                                ("Ctrl+A", "Select all"),
                                ("Ctrl+F", "Search"),
                                ("Ctrl+O", "Add file"),
                                ("M", "Magnet"),
                                ("Space", "Pause / resume"),
                                ("Delete", "Delete"),
                                ("B", "Toggle sidebar"),
                                ("Ctrl+,", "Settings"),
                                ("Ctrl+L", "Logs"),
                            ] {
                                ui.label(RichText::new(k).monospace().strong());
                                ui.label(v);
                                ui.end_row();
                            }
                        });
                });
            if matches!(result, DialogResult::Closed(_)) {
                self.dialogs.shortcuts = false;
            }
        }
    }

    fn any_dialog(&self) -> bool {
        let d = &self.dialogs;
        d.magnet.is_some()
            || d.add.is_some()
            || d.upload.is_some()
            || d.delete.is_some()
            || d.settings.is_some()
    }

    /// Global shortcuts, when no text field has focus and no dialog is open.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.any_dialog() || ctx.egui_wants_keyboard_input() || self.screen != Screen::Torrents {
            return;
        }
        let (comma, l, m, b, del, q) = ctx.input(|i| {
            (
                i.modifiers.command && i.key_pressed(egui::Key::Comma),
                i.modifiers.command && i.key_pressed(egui::Key::L),
                !i.modifiers.any() && i.key_pressed(egui::Key::M),
                !i.modifiers.any() && i.key_pressed(egui::Key::B),
                i.key_pressed(egui::Key::Delete),
                i.key_pressed(egui::Key::Questionmark),
            )
        });
        if comma {
            self.dialogs.settings = Some(self.settings.clone());
        }
        if l {
            self.screen = Screen::Logs;
        }
        if m {
            self.dialogs.magnet = Some(String::new());
        }
        if b {
            self.sidebar_collapsed = !self.sidebar_collapsed;
        }
        if del && !self.grid.selection.selected.is_empty() {
            self.dialogs.delete = Some(self.selected_ids());
        }
        if q {
            self.dialogs.shortcuts = true;
        }
    }

    /// For tests: the current theme kind.
    pub fn theme_kind(&self) -> ThemeKind {
        if self.dark {
            ThemeKind::Dark
        } else {
            ThemeKind::Light
        }
    }

    /// For tests: fill in the login password.
    pub fn set_password(&mut self, password: &str) {
        self.login.password = password.to_owned();
    }

    /// For tests: switch the open settings dialog to `tab`.
    pub fn set_settings_tab(&mut self, tab: usize) {
        if let Some(s) = &mut self.dialogs.settings {
            s.tab = tab;
        }
    }

    /// For tests: whether the settings dialog is open.
    pub fn settings_open(&self) -> bool {
        self.dialogs.settings.is_some()
    }

    /// For tests: the current screen.
    pub fn screen(&self) -> Screen {
        self.screen
    }
}

fn nz(s: String) -> String {
    if s.is_empty() { "0 B/s".to_owned() } else { s }
}

impl eframe::App for MockupApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}
