//! Screen state and the connect form. Login runs off the UI thread.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use fluent2_egui::controls::{Appearance, Button, Input};
use fluent2_egui::data::DataGridState;
use fluent2_egui::data::tree_view::TreeState;
use fluent2_egui::office::status_bar::StatusBar;
use fluent2_egui::office::{TitleBar, WindowAction, window_command};
use fluent2_egui::theme::{Theme, install};
use fluent2_egui::tokens::FLUENT;
use rtbit_client::{AuthStatus, Error, RssRuleBody, RssRuleEpisode, RssSnapshot, SettingsSnapshot};

use crate::screens::{
    AddDraft, AddRequest, DetailAction, DetailTab, DetailView, Episodes, RowAction, RssAction,
    RssView, RuleEditor, SettingsChanges, SettingsDraft, SettingsTab,
};

use crate::keychain::{OsKeychain, TokenStore};
use crate::screens::{draw_detail, draw_main};
use crate::state::{ANY, ListCache, StatusFilter};
pub use rtbit_ui_core::{Connection, DaemonLogin, DetailSnapshot, LoginAttempt, Screen, Snapshot};

/// How often the main screen asks the daemon for the list and the session speeds.
const POLL_EVERY: Duration = Duration::from_secs(2);

struct PendingLogin {
    rx: Receiver<Result<Option<String>, String>>,
    /// `true` when this worker is a reconnect, so a failure stays on Connect
    /// without a scary first-run error if the token was simply missing.
    reconnect: bool,
}

/// The shell: chrome, the current screen, and one in-flight login.
pub struct App<L: LoginAttempt = DaemonLogin, S: TokenStore = OsKeychain> {
    screen: Screen,
    connection: Connection,
    url: String,
    username: String,
    password: String,
    setup_token: String,
    confirm_password: String,
    /// Shown under the form after a failed attempt. Empty when there is none.
    error: String,
    /// Dismissible error bar under the title bar. Empty when there is none.
    notice: String,
    /// Refresh token last written to the keychain, so a rotation is stored once.
    stored_refresh: Option<String>,
    /// Status check in flight before the connect screen decides on setup.
    status_flight: Option<Receiver<Result<AuthStatus, String>>>,
    /// True once `/auth/status` has answered for this visit to Connect.
    status_known: bool,
    login: L,
    keychain: S,
    pending: Option<PendingLogin>,
    /// One reconnect attempt per launch. A failure leaves the connect screen.
    reconnect_tried: bool,
    theme_ready: bool,
    cache: ListCache,
    status_filter: StatusFilter,
    category: String,
    tracker: String,
    grid: DataGridState,
    poll: Option<Poll>,
    poll_at: Option<Instant>,
    /// Torrent the command bar acts on. None until a row is selected.
    selected: Option<u64>,
    /// Delete waits for the confirm dialog.
    confirm_delete: bool,
    /// The add dialog.
    add_open: bool,
    add_draft: AddDraft,
    action: Option<ActionFlight>,
    /// An add is in flight. The bar stays quiet until it finishes.
    add_flight: Option<Receiver<Result<(), String>>>,
    /// Selected torrent's files, trackers, peers and pieces.
    detail: DetailView,
    detail_tab: DetailTab,
    /// Expanded folders in the file tree. The shell owns it.
    file_tree: TreeState<i64>,
    tracker_url: String,
    detail_poll: Option<DetailPoll>,
    detail_poll_at: Option<Instant>,
    /// Torrent the in-flight detail poll is for. A new selection drops it.
    detail_for: Option<u64>,
    detail_flight: Option<Receiver<Result<(), String>>>,
    rss_view: RssView,
    rss_load: Option<Receiver<Result<RssSnapshot, String>>>,
    rss_download: Option<Receiver<Result<(), String>>>,
    rss_error: String,
    rss_editor: RuleEditor,
    /// A rule save (`true`) or delete (`false`) in flight.
    rss_rule_op: Option<(bool, Receiver<Result<(), String>>)>,
    /// The episode history read for the rule being edited, by rule id.
    rss_episodes_load: Option<(String, EpisodesReceiver)>,
    settings_tab: SettingsTab,
    settings_draft: SettingsDraft,
    settings_load: Option<Receiver<Result<SettingsSnapshot, String>>>,
    settings_save: Option<Receiver<Result<(), String>>>,
    settings_error: String,
    /// Shown on the network form once the reload after a network save lands.
    settings_notice: String,
}

impl Default for App<DaemonLogin> {
    fn default() -> Self {
        Self::new()
    }
}

impl App<DaemonLogin> {
    #[must_use]
    pub fn new() -> Self {
        Self::with_login(DaemonLogin)
    }
}

impl<L: LoginAttempt + Clone + 'static, S: TokenStore> App<L, S> {
    /// Same shell with a chosen token store. Tests use the memory store so
    /// they never touch the OS keychain.
    #[must_use]
    pub fn with_store(login: L, keychain: S) -> Self {
        Self {
            screen: Screen::Connect,
            connection: Connection::Disconnected,
            url: "http://127.0.0.1:3030".to_owned(),
            username: String::new(),
            password: String::new(),
            setup_token: String::new(),
            confirm_password: String::new(),
            error: String::new(),
            notice: String::new(),
            stored_refresh: None,
            status_flight: None,
            status_known: false,
            login,
            keychain,
            pending: None,
            reconnect_tried: false,
            theme_ready: false,
            cache: ListCache::default(),
            status_filter: StatusFilter::default(),
            category: ANY.to_owned(),
            tracker: ANY.to_owned(),
            grid: DataGridState::default(),
            poll: None,
            poll_at: None,
            selected: None,
            confirm_delete: false,
            add_open: false,
            add_draft: AddDraft::default(),
            action: None,
            add_flight: None,
            detail: DetailView::default(),
            detail_tab: DetailTab::Files,
            file_tree: TreeState::default(),
            tracker_url: String::new(),
            detail_poll: None,
            detail_poll_at: None,
            detail_for: None,
            detail_flight: None,
            rss_view: RssView::default(),
            rss_load: None,
            rss_download: None,
            rss_error: String::new(),
            rss_editor: RuleEditor::default(),
            rss_episodes_load: None,
            rss_rule_op: None,
            settings_tab: SettingsTab::General,
            settings_draft: SettingsDraft::default(),
            settings_load: None,
            settings_save: None,
            settings_error: String::new(),
            settings_notice: String::new(),
        }
    }
}

impl<L: LoginAttempt + Clone + 'static> App<L> {
    /// `login` is cloned onto the worker thread, so it must not hold the UI.
    #[must_use]
    pub fn with_login(login: L) -> App<L, OsKeychain> {
        Self {
            screen: Screen::Connect,
            connection: Connection::Disconnected,
            url: "http://127.0.0.1:3030".to_owned(),
            username: String::new(),
            password: String::new(),
            setup_token: String::new(),
            confirm_password: String::new(),
            error: String::new(),
            notice: String::new(),
            stored_refresh: None,
            status_flight: None,
            status_known: false,
            login,
            keychain: OsKeychain,
            pending: None,
            reconnect_tried: false,
            theme_ready: false,
            cache: ListCache::default(),
            status_filter: StatusFilter::default(),
            category: ANY.to_owned(),
            tracker: ANY.to_owned(),
            grid: DataGridState::default(),
            poll: None,
            poll_at: None,
            selected: None,
            confirm_delete: false,
            add_open: false,
            add_draft: AddDraft::default(),
            action: None,
            add_flight: None,
            detail: DetailView::default(),
            detail_tab: DetailTab::Files,
            file_tree: TreeState::default(),
            tracker_url: String::new(),
            detail_poll: None,
            detail_poll_at: None,
            detail_for: None,
            detail_flight: None,
            rss_view: RssView::default(),
            rss_load: None,
            rss_download: None,
            rss_error: String::new(),
            rss_editor: RuleEditor::default(),
            rss_episodes_load: None,
            rss_rule_op: None,
            settings_tab: SettingsTab::General,
            settings_draft: SettingsDraft::default(),
            settings_load: None,
            settings_save: None,
            settings_error: String::new(),
            settings_notice: String::new(),
        }
    }
}

impl<L: LoginAttempt + Clone + 'static, S: TokenStore> App<L, S> {
    #[must_use]
    pub fn screen(&self) -> Screen {
        self.screen
    }

    #[must_use]
    pub fn connection(&self) -> Connection {
        self.connection
    }

    #[must_use]
    pub fn error(&self) -> &str {
        &self.error
    }

    /// Record a message the user can read, on the screen and in the bar.
    fn fail(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.notice = message.clone();
        self.error = message;
    }

    /// Test hook: show the main screen with an empty list and no poll.
    pub fn note_empty_for_test(&mut self) {
        self.screen = Screen::Main;
        self.connection = Connection::Connected;
        self.cache.torrents.clear();
        self.poll_at = Some(std::time::Instant::now());
    }

    /// Test hook: select a torrent so the command bar has a row to act on.
    pub fn select_for_test(&mut self, id: u64) {
        self.selected = Some(id);
    }

    /// Start a login on a worker thread. A second click while one is in
    /// flight does nothing. The password moves with the worker and is not logged.
    pub fn submit(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let login = self.login.clone();
        let url = self.url.clone();
        let username = self.username.clone();
        let password = self.password.clone();
        let (tx, rx) = mpsc::channel();
        self.connection = Connection::Connecting;
        self.error.clear();
        self.pending = Some(PendingLogin {
            rx,
            reconnect: false,
        });
        std::thread::spawn(move || {
            let outcome = login
                .login(&url, &username, &password)
                .map_err(|err| login_message(&err));
            let _ = tx.send(outcome);
        });
    }

    /// One try, on the first frame, to continue from the keychain. No token,
    /// or a keychain that cannot be opened, leaves the connect screen.
    fn reconnect(&mut self) {
        if self.reconnect_tried
            || self.pending.is_some()
            || self.screen != Screen::Connect
            || !self.status_known
        {
            return;
        }
        self.reconnect_tried = true;
        let loaded = self.keychain.load(&self.url);
        let Some(Some(refresh)) = loaded else { return };
        let login = self.login.clone();
        let url = self.url.clone();
        let (tx, rx) = mpsc::channel();
        self.connection = Connection::Connecting;
        self.pending = Some(PendingLogin {
            rx,
            reconnect: true,
        });
        std::thread::spawn(move || {
            let outcome = login
                .resume(&url, &refresh)
                .map_err(|err| login_message(&err));
            let _ = tx.send(outcome);
        });
    }

    /// Write a refresh token when it is not the one already stored.
    fn remember(&mut self, token: &str) {
        if self.stored_refresh.as_deref() == Some(token) {
            return;
        }
        // A keychain that refuses the write does not fail the session.
        // The token simply will not survive a relaunch.
        let _ = self.keychain.store(&self.url, token);
        self.stored_refresh = Some(token.to_owned());
    }

    /// After a poll or an action the client may have rotated the refresh
    /// token on a 401. Write the new one back; never log it.
    fn persist_rotation(&mut self) {
        if self.screen != Screen::Main {
            return;
        }
        let Some(token) = self.login.rotated_refresh() else {
            return;
        };
        self.remember(&token);
    }

    /// Leave the session and drop the stored refresh token.
    pub fn logout(&mut self) {
        self.login.logout();
        let _ = self.keychain.clear(&self.url);
        self.stored_refresh = None;
        self.status_known = false;
        self.status_flight = None;
        self.screen = Screen::Connect;
        self.connection = Connection::Disconnected;
        self.password.clear();
        self.selected = None;
        self.cache.torrents.clear();
        self.poll = None;
        self.detail = DetailView::default();
    }

    /// Take a finished login, if the worker has sent one.
    pub fn poll_login(&mut self) {
        let finished = self
            .pending
            .as_ref()
            .and_then(|pending| pending.rx.try_recv().ok());
        if let Some(outcome) = finished {
            let reconnect = self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.reconnect);
            self.pending = None;
            match outcome {
                Ok(refresh) => {
                    if let Some(token) = refresh.as_deref() {
                        self.remember(token);
                    }
                    self.connection = Connection::Connected;
                    self.screen = Screen::Main;
                    self.error.clear();
                    self.password.clear();
                    self.poll_at = None;
                }
                Err(message) => {
                    self.connection = Connection::Error;
                    self.screen = Screen::Connect;
                    if reconnect {
                        let _ = self.keychain.clear(&self.url);
                        self.fail("Saved session expired. Sign in again.");
                    } else {
                        self.fail(message);
                    }
                }
            }
        }
    }

    /// Draw one frame. Tests call this through `egui_kittest` without a window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if !self.theme_ready {
            install(ui.ctx(), &Theme::dark(FLUENT));
            self.theme_ready = true;
        }
        self.ask_status();
        self.poll_status();
        self.reconnect();
        self.poll_login();
        self.poll_list();
        self.poll_detail();
        self.persist_rotation();
        self.chrome(ui);
    }

    fn chrome(&mut self, ui: &mut egui::Ui) {
        let maximized = ui.input(|input| input.viewport().maximized.unwrap_or(false));
        let focused = ui.input(|input| input.viewport().focused);
        let response = TitleBar::new("rtbit_title", "rustTorrent")
            .maximized(maximized)
            .focused(focused)
            .show(ui, |_| ());
        if let Some(action) = response.action {
            ui.ctx()
                .send_viewport_cmd(window_command(action, maximized));
        }
        if response.double_clicked {
            ui.ctx()
                .send_viewport_cmd(window_command(WindowAction::ToggleMaximize, maximized));
        }

        let reserved = 24.0;
        let body = (ui.available_height() - reserved).max(0.0);
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), body),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                if !self.notice.is_empty() {
                    let text = self.notice.clone();
                    let bar = fluent2_egui::surfaces::MessageBar::new(
                        fluent2_egui::surfaces::Intent::Error,
                        text,
                    )
                    .dismissible()
                    .show(ui);
                    if bar.dismissed {
                        self.notice.clear();
                    }
                }
                match self.screen {
                    Screen::Connect => self.connect_screen(ui),
                    Screen::Setup => self.setup_screen(ui),
                    Screen::Main => self.main_screen(ui),
                    Screen::Settings => self.settings_screen(ui),
                    Screen::Rss => self.rss_screen(ui),
                }
            },
        );

        let status = self.connection.label().to_owned();
        StatusBar::new("rtbit_status")
            .left(move |ui| {
                ui.label(status);
            })
            .show(ui);
    }

    /// Ask whether setup is still open, once, before reconnecting.
    fn ask_status(&mut self) {
        if self.status_known || self.status_flight.is_some() || self.screen != Screen::Connect {
            return;
        }
        let login = self.login.clone();
        let url = self.url.clone();
        let (tx, rx) = mpsc::channel();
        self.status_flight = Some(rx);
        std::thread::spawn(move || {
            let outcome = login.status(&url).map_err(|err| login_message(&err));
            let _ = tx.send(outcome);
        });
    }

    fn poll_status(&mut self) {
        let finished = self
            .status_flight
            .as_ref()
            .and_then(|rx| rx.try_recv().ok());
        let Some(outcome) = finished else { return };
        self.status_flight = None;
        self.status_known = true;
        match outcome {
            Ok(status) if status.setup_required && self.screen == Screen::Connect => {
                self.screen = Screen::Setup;
                self.error.clear();
            }
            Ok(_) => {}
            Err(message) => {
                // Unreachable daemon stays on Connect. Do not invent a setup screen.
                self.connection = Connection::Error;
                self.fail(message);
            }
        }
    }

    /// Client-side checks, then the daemon's own setup validation.
    pub fn submit_setup(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let token = self.setup_token.trim();
        let username = self.username.trim();
        if token.is_empty() || username.is_empty() || self.password.is_empty() {
            self.fail("Setup token, username and password are required.");
            return;
        }
        if self.password.chars().count() < 4 {
            self.fail("Password must be at least 4 characters.");
            return;
        }
        if self.password != self.confirm_password {
            self.fail("Passwords do not match.");
            return;
        }
        let login = self.login.clone();
        let url = self.url.clone();
        let username = username.to_owned();
        let password = self.password.clone();
        let setup_token = token.to_owned();
        let (tx, rx) = mpsc::channel();
        self.connection = Connection::Connecting;
        self.error.clear();
        self.pending = Some(PendingLogin {
            rx,
            reconnect: false,
        });
        std::thread::spawn(move || {
            let outcome = login
                .setup(&url, &username, &password, &setup_token)
                .map_err(|err| login_message(&err));
            let _ = tx.send(outcome);
        });
    }

    fn setup_screen(&mut self, ui: &mut egui::Ui) {
        ui.add_space(24.0);
        ui.heading("Set up");
        ui.label("The setup token is printed in the rtbit log.");
        ui.add_space(8.0);
        ui.add(
            Input::new(&mut self.setup_token)
                .label("Setup token")
                .password()
                .id_salt("setup_token"),
        );
        ui.add_space(8.0);
        ui.add(
            Input::new(&mut self.username)
                .label("Username")
                .id_salt("setup_username"),
        );
        ui.add_space(8.0);
        ui.add(
            Input::new(&mut self.password)
                .label("Password")
                .password()
                .id_salt("setup_password"),
        );
        ui.add_space(8.0);
        ui.add(
            Input::new(&mut self.confirm_password)
                .label("Confirm password")
                .password()
                .id_salt("setup_confirm"),
        );
        ui.add_space(12.0);
        let busy = self.pending.is_some();
        if ui
            .add_enabled(
                !busy,
                Button::new("Create account").appearance(Appearance::Primary),
            )
            .clicked()
        {
            self.submit_setup();
        }
        if !self.error.is_empty() {
            ui.add_space(8.0);
            ui.colored_label(egui::Color32::from_rgb(196, 43, 28), &self.error);
        }
    }

    fn connect_screen(&mut self, ui: &mut egui::Ui) {
        ui.add_space(24.0);
        ui.heading("Connect");
        ui.add_space(8.0);
        ui.add(
            Input::new(&mut self.url)
                .label("Daemon URL")
                .id_salt("daemon_url"),
        );
        ui.add_space(8.0);
        ui.add(
            Input::new(&mut self.username)
                .label("Username")
                .id_salt("username"),
        );
        ui.add_space(8.0);
        ui.add(
            Input::new(&mut self.password)
                .label("Password")
                .password()
                .id_salt("password"),
        );
        ui.add_space(12.0);
        let busy = self.pending.is_some();
        if ui
            .add_enabled(
                !busy,
                Button::new("Connect").appearance(Appearance::Primary),
            )
            .clicked()
        {
            self.submit();
        }
        if !self.error.is_empty() {
            ui.add_space(8.0);
            ui.colored_label(egui::Color32::from_rgb(196, 43, 28), &self.error);
        }
    }

    fn main_screen(&mut self, ui: &mut egui::Ui) {
        self.take_action();
        ui.horizontal(|ui| {
            ui.label("Session");
            if ui.button("Settings").clicked() && self.screen == Screen::Main {
                self.open_settings();
            }
            if ui.button("RSS").clicked() && self.screen == Screen::Main {
                self.open_rss();
            }
            if ui.button("Sign out").clicked() {
                self.logout();
            }
        });
        // The grid's central panel takes everything left in the frame. Keep a
        // strip under it for the detail pane, which would otherwise be laid
        // out past the window.
        let detail_height = 220.0;
        let grid_height = (ui.available_height() - detail_height).max(120.0);
        let chosen = ui
            .allocate_ui_with_layout(
                egui::vec2(ui.available_width(), grid_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    draw_main(
                        ui,
                        &self.cache,
                        &mut self.status_filter,
                        &mut self.category,
                        &mut self.tracker,
                        &mut self.grid,
                        crate::screens::RowActions {
                            selected: self.selected,
                            confirm_delete: &mut self.confirm_delete,
                            add_open: &mut self.add_open,
                            draft: &mut self.add_draft,
                            busy: self.action.is_some() || self.add_flight.is_some(),
                        },
                    )
                },
            )
            .inner;
        if let Some(id) = chosen.selected {
            self.selected = Some(id);
        }
        if let Some(action) = chosen.action {
            self.dispatch(action);
        }
        if let Some(request) = chosen.add {
            self.dispatch_add(request);
        }
        self.poll_add();
        // A nested Panel::bottom lays out past the frame, so the tabs were
        // drawn outside the window and could not be clicked. Take the strip
        // from the space under the grid instead.
        ui.add_space(8.0);
        ui.separator();
        self.draw_detail_pane(ui);
    }

    /// The pane sits under the grid so a selected row's files stay visible.
    fn draw_detail_pane(&mut self, ui: &mut egui::Ui) {
        self.take_detail_action();
        if self.selected.is_none() {
            self.detail = DetailView::default();
        }
        let action = draw_detail(
            ui,
            &self.detail,
            &mut self.detail_tab,
            &mut self.tracker_url,
            &mut self.file_tree,
            self.detail_flight.is_some(),
        );
        if let Some(action) = action {
            self.dispatch_detail(action);
        }
    }

    fn dispatch_detail(&mut self, action: DetailAction) {
        if self.detail_flight.is_some() {
            return;
        }
        let Some(id) = self.selected else { return };
        let login = self.login.clone();
        let (tx, rx) = mpsc::channel();
        let added = matches!(action, DetailAction::AddTracker(_));
        std::thread::spawn(move || {
            let outcome = login
                .detail_act(id, &action)
                .map_err(|err| login_message(&err));
            let _ = tx.send(outcome);
        });
        self.detail_flight = Some(rx);
        if added {
            self.tracker_url.clear();
        }
    }

    fn take_detail_action(&mut self) {
        let Some(rx) = &self.detail_flight else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(())) => {
                self.detail_flight = None;
                self.detail_poll_at = None;
            }
            Ok(Err(message)) => {
                self.detail.error = Some(message);
                self.detail_flight = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.detail_flight = None,
        }
    }

    /// Detail endpoints only for the selected torrent, on the same interval
    /// as the list. A changed selection drops the previous pane.
    fn poll_detail(&mut self) {
        self.take_detail_poll();
        if self.screen != Screen::Main {
            return;
        }
        let Some(id) = self.selected else {
            self.detail = DetailView::default();
            self.detail_for = None;
            return;
        };
        if self.detail_for != Some(id) {
            self.detail = DetailView::default();
            self.detail_for = Some(id);
            self.detail_poll = None;
            self.detail_poll_at = None;
        }
        if self.detail_poll.is_some() {
            return;
        }
        let due = self
            .detail_poll_at
            .is_none_or(|at| at.elapsed() >= POLL_EVERY);
        if !due {
            return;
        }
        let login = self.login.clone();
        let (tx, rx) = mpsc::channel();
        self.detail_poll = Some(DetailPoll { rx, id });
        std::thread::spawn(move || {
            let outcome = login.detail(id).map_err(|err| login_message(&err));
            let _ = tx.send(outcome);
        });
    }

    fn take_detail_poll(&mut self) {
        let finished = self
            .detail_poll
            .as_ref()
            .and_then(|poll| poll.rx.try_recv().ok().map(|outcome| (poll.id, outcome)));
        let Some((id, outcome)) = finished else {
            return;
        };
        self.detail_poll = None;
        self.detail_poll_at = Some(Instant::now());
        if self.selected != Some(id) {
            return;
        }
        match outcome {
            Ok(snapshot) => {
                self.detail.torrent = Some(snapshot.torrent);
                self.detail.trackers = Some(snapshot.trackers);
                self.detail.peers = Some(snapshot.peers);
                self.detail.pieces = Some(snapshot.pieces);
                self.detail.error = None;
            }
            Err(message) => self.detail.error = Some(message),
        }
    }

    /// Post one add. The dialog already checked the magnet and the path.
    fn dispatch_add(&mut self, request: AddRequest) {
        if self.add_flight.is_some() {
            return;
        }
        let login = self.login.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = login.add(&request).map_err(|err| login_message(&err));
            let _ = tx.send(result);
        });
        self.add_flight = Some(rx);
        self.add_draft = AddDraft::default();
    }

    fn poll_add(&mut self) {
        let Some(rx) = &self.add_flight else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(())) => self.add_flight = None,
            Ok(Err(message)) => {
                self.add_draft.error = message;
                self.add_open = true;
                self.add_flight = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.add_flight = None,
        }
    }

    /// Post one row action. Delete is only reached after the dialog confirms.
    fn dispatch(&mut self, action: RowAction) {
        if self.action.is_some() {
            return;
        }
        let Some(id) = self.selected else { return };
        let login = self.login.clone();
        let (tx, rx) = mpsc::channel();
        self.action = Some(ActionFlight { rx, action });
        std::thread::spawn(move || {
            let outcome = login.act(id, action).map_err(|err| login_message(&err));
            let _ = tx.send(outcome);
        });
    }

    fn take_action(&mut self) {
        let finished = self
            .action
            .as_ref()
            .and_then(|flight| flight.rx.try_recv().ok());
        let Some((outcome, action)) =
            finished.and_then(|outcome| self.action.take().map(|flight| (outcome, flight.action)))
        else {
            return;
        };
        match outcome {
            Ok(()) => {
                self.cache.error = None;
                if action == RowAction::Delete {
                    if let Some(id) = self.selected {
                        self.cache.torrents.retain(|torrent| torrent.id != Some(id));
                    }
                    self.selected = None;
                }
                // The next frame polls, so the row shows the daemon's new state.
                self.poll_at = None;
            }
            Err(message) => {
                self.fail(message.clone());
                self.cache.error = Some(message);
            }
        }
    }

    /// Start a poll when the interval has elapsed and none is in flight.
    fn poll_list(&mut self) {
        self.take_poll();
        if self.screen != Screen::Main || self.poll.is_some() {
            return;
        }
        let due = self.poll_at.is_none_or(|at| at.elapsed() >= POLL_EVERY);
        if !due {
            return;
        }
        let login = self.login.clone();
        let (tx, rx) = mpsc::channel();
        self.poll = Some(Poll { rx });
        std::thread::spawn(move || {
            let outcome = login.snapshot().map_err(|err| login_message(&err));
            let _ = tx.send(outcome);
        });
    }

    fn take_poll(&mut self) {
        let finished = self.poll.as_ref().and_then(|poll| poll.rx.try_recv().ok());
        if let Some(outcome) = finished {
            self.poll = None;
            self.poll_at = Some(Instant::now());
            match outcome {
                Ok(snapshot) => {
                    self.cache.torrents = snapshot.torrents.torrents;
                    self.cache.stats = Some(snapshot.stats);
                    self.cache.error = None;
                    self.connection = Connection::Connected;
                }
                Err(message) => {
                    self.fail(message.clone());
                    self.cache.error = Some(message);
                }
            }
        }
    }

    fn open_settings(&mut self) {
        if self.settings_load.is_some() {
            self.screen = Screen::Settings;
            return;
        }
        self.screen = Screen::Settings;
        self.settings_error.clear();
        let login = self.login.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let outcome = login.load_settings().map_err(|err| login_message(&err));
            let _ = tx.send(outcome);
        });
        self.settings_load = Some(rx);
    }

    fn settings_screen(&mut self, ui: &mut egui::Ui) {
        self.take_settings();
        ui.horizontal(|ui| {
            if ui.button("Back").clicked() {
                self.screen = Screen::Main;
            }
        });
        let action = crate::screens::draw_settings(
            ui,
            &mut self.settings_tab,
            &mut self.settings_draft,
            self.settings_save.is_some() || self.settings_load.is_some(),
            &self.settings_error,
        );
        match action {
            crate::screens::SettingsAction::Save => self.submit_settings(),
            crate::screens::SettingsAction::UseStartupFlags => {
                self.send_settings(SettingsChanges::use_startup_flags());
            }
            crate::screens::SettingsAction::None => {}
        }
    }

    fn take_settings(&mut self) {
        if let Some(rx) = &self.settings_load {
            match rx.try_recv() {
                Ok(Ok(snap)) => {
                    self.settings_draft = SettingsDraft::from_snapshot(&snap);
                    self.settings_draft.network_notice = std::mem::take(&mut self.settings_notice);
                    self.settings_load = None;
                    self.settings_error.clear();
                }
                Ok(Err(message)) => {
                    self.fail(message.clone());
                    self.settings_error = message;
                    self.settings_load = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.settings_load = None,
            }
        }
        if let Some(rx) = &self.settings_save {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    self.settings_save = None;
                    self.settings_error.clear();
                    self.open_settings();
                }
                Ok(Err(message)) => {
                    self.fail(message.clone());
                    self.settings_error = message;
                    self.settings_save = None;
                    self.settings_notice.clear();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.settings_save = None,
            }
        }
    }

    fn submit_settings(&mut self) {
        if self.settings_save.is_some() {
            return;
        }
        let changes = match self.settings_draft.changes() {
            Ok(changes) if changes.is_empty() => {
                self.settings_error.clear();
                return;
            }
            Ok(changes) => changes,
            Err(message) => {
                self.fail(message.clone());
                self.settings_error = message;
                return;
            }
        };
        self.send_settings(changes);
    }

    fn send_settings(&mut self, changes: SettingsChanges) {
        if self.settings_save.is_some() {
            return;
        }
        self.settings_notice = match &changes.network {
            Some(body) if body.reset == Some(true) && body.listen_port.is_none() => {
                "Saved values cleared. The next start uses the startup flags.".into()
            }
            Some(_) => "Saved. Restart to apply.".into(),
            None => String::new(),
        };
        let login = self.login.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let outcome = login
                .save_settings(&changes)
                .map_err(|err| login_message(&err));
            let _ = tx.send(outcome);
        });
        self.settings_save = Some(rx);
        self.settings_error.clear();
    }
    fn open_rss(&mut self) {
        self.screen = Screen::Rss;
        self.rss_error.clear();
        if self.rss_load.is_some() {
            return;
        }
        let login = self.login.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let outcome = login.load_rss().map_err(|err| login_message(&err));
            let _ = tx.send(outcome);
        });
        self.rss_load = Some(rx);
    }

    fn rss_screen(&mut self, ui: &mut egui::Ui) {
        self.take_rss();
        ui.horizontal(|ui| {
            if ui.button("Back").clicked() {
                self.screen = Screen::Main;
            }
        });
        let busy =
            self.rss_load.is_some() || self.rss_download.is_some() || self.rss_rule_op.is_some();
        match crate::screens::draw_rss(
            ui,
            &self.rss_view,
            &mut self.rss_editor,
            busy,
            &self.rss_error,
        ) {
            Some(RssAction::Download(id)) => self.download_rss(&id),
            Some(RssAction::SaveRule { id, body }) => self.save_rss_rule(id, body),
            Some(RssAction::DeleteRule(id)) => self.delete_rss_rule(id),
            Some(RssAction::LoadEpisodes(id)) => self.load_rule_episodes(id),
            None => {}
        }
    }

    fn take_rss(&mut self) {
        self.take_rule_episodes();
        if let Some(rx) = &self.rss_load {
            match rx.try_recv() {
                Ok(Ok(snap)) => {
                    self.rss_view = RssView::from_snapshot(&snap);
                    self.rss_load = None;
                    self.rss_error.clear();
                }
                Ok(Err(message)) => {
                    self.fail(message.clone());
                    self.rss_error = message;
                    self.rss_load = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.rss_load = None,
            }
        }
        if let Some(rx) = &self.rss_download {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    self.rss_download = None;
                    self.rss_error.clear();
                    self.rss_load = None;
                    self.open_rss();
                }
                Ok(Err(message)) => {
                    self.fail(message.clone());
                    self.rss_error = message;
                    self.rss_download = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.rss_download = None,
            }
        }
        if let Some((save, rx)) = &self.rss_rule_op {
            let save = *save;
            match rx.try_recv() {
                Ok(Ok(())) => {
                    self.rss_rule_op = None;
                    if save {
                        self.rss_editor.saved();
                    }
                    self.rss_error.clear();
                    self.open_rss();
                }
                Ok(Err(message)) => {
                    self.rss_rule_op = None;
                    if save {
                        self.rss_editor.refused(message);
                    } else {
                        self.rss_error = message;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.rss_rule_op = None,
            }
        }
    }

    fn load_rule_episodes(&mut self, id: String) {
        let login = self.login.clone();
        let (tx, rx) = mpsc::channel();
        let rule = id.clone();
        std::thread::spawn(move || {
            let outcome = login
                .rss_rule_episodes(&rule)
                .map_err(|err| login_message(&err));
            let _ = tx.send(outcome);
        });
        // A later Edit replaces an earlier read; its answer is dropped.
        self.rss_episodes_load = Some((id, rx));
    }

    fn take_rule_episodes(&mut self) {
        let Some((id, rx)) = &self.rss_episodes_load else {
            return;
        };
        let outcome = match rx.try_recv() {
            Ok(outcome) => outcome,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("Episode history unavailable".to_owned())
            }
        };
        if self.rss_editor.draft.id.as_deref() == Some(id.as_str()) {
            self.rss_editor.episodes = match outcome {
                Ok(list) => Episodes::Loaded(list),
                Err(message) => Episodes::Failed(message),
            };
        }
        self.rss_episodes_load = None;
    }

    fn save_rss_rule(&mut self, id: Option<String>, body: RssRuleBody) {
        if self.rss_rule_op.is_some() {
            return;
        }
        let login = self.login.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let outcome = login
                .save_rss_rule(id.as_deref(), &body)
                .map_err(|err| login_message(&err));
            let _ = tx.send(outcome);
        });
        self.rss_rule_op = Some((true, rx));
    }

    fn delete_rss_rule(&mut self, id: String) {
        if self.rss_rule_op.is_some() {
            return;
        }
        let login = self.login.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let outcome = login
                .delete_rss_rule(&id)
                .map_err(|err| login_message(&err));
            let _ = tx.send(outcome);
        });
        self.rss_rule_op = Some((false, rx));
        self.rss_error.clear();
    }

    fn download_rss(&mut self, id: &str) {
        if self.rss_download.is_some() {
            return;
        }
        let id = id.to_owned();
        let login = self.login.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let outcome = login.download_rss(&id).map_err(|err| login_message(&err));
            let _ = tx.send(outcome);
        });
        self.rss_download = Some(rx);
        self.rss_error.clear();
    }
}

/// Answer to one rule's episode-history read.
type EpisodesReceiver = Receiver<Result<Vec<RssRuleEpisode>, String>>;

struct Poll {
    rx: Receiver<Result<Snapshot, String>>,
}

struct DetailPoll {
    rx: Receiver<Result<DetailSnapshot, String>>,
    id: u64,
}

struct ActionFlight {
    rx: Receiver<Result<(), String>>,
    action: RowAction,
}

fn login_message(err: &Error) -> String {
    crate::map_error(err)
}
