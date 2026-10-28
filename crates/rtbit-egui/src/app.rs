//! Screen state and the connect form. Login runs off the UI thread.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use fluent2_egui::controls::{Appearance, Button, Input};
use fluent2_egui::data::DataGridState;
use fluent2_egui::office::status_bar::StatusBar;
use fluent2_egui::office::{TitleBar, WindowAction, window_command};
use fluent2_egui::theme::{Theme, install};
use fluent2_egui::tokens::FLUENT;
use rtbit_client::{Client, Error, SessionStats, TorrentList};

use crate::screens::{AddDraft, AddRequest, AddSource, RowAction};

use crate::screens::draw_main;
use crate::state::{ANY, ListCache, StatusFilter};

/// How often the main screen asks the daemon for the list and the session speeds.
const POLL_EVERY: Duration = Duration::from_secs(2);

/// Which screen is showing. Later items add setup, detail, add and settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    /// Daemon URL, username and password.
    Connect,
    /// Placeholder until the torrent grid lands.
    Main,
}

/// What the status bar says about the daemon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

impl Connection {
    fn label(self) -> &'static str {
        match self {
            Self::Disconnected => "Disconnected",
            Self::Connecting => "Connecting",
            Self::Connected => "Connected",
            Self::Error => "Error",
        }
    }
}

/// One poll of the main screen: the torrent list (with stats) and session speeds.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub torrents: TorrentList,
    pub stats: SessionStats,
}

/// How a connect click reaches the daemon. Tests substitute a value that
/// never opens a socket. `snapshot` is what the 2-second poll calls.
///
/// A session is opened once by `login` and then reused. `snapshot` and `act`
/// must not log in again; the client refreshes its own token on a 401.
pub trait LoginAttempt: Send {
    /// Try the credentials and keep the session. The password must not be written to a log.
    fn login(&self, url: &str, username: &str, password: &str) -> Result<(), Error>;

    /// The current list and session speeds, on the session `login` opened.
    fn snapshot(&self) -> Result<Snapshot, Error> {
        Err(Error::Unreachable("no snapshot".into()))
    }

    /// Start, pause or delete one torrent on the session `login` opened.
    fn act(&self, id: u64, action: RowAction) -> Result<(), Error> {
        let _ = (id, action);
        Err(Error::Unreachable("no action".into()))
    }

    /// Add a magnet or a torrent file on the session `login` opened.
    fn add(&self, request: &AddRequest) -> Result<(), Error> {
        let _ = request;
        Err(Error::Unreachable("no add".into()))
    }
}

/// The real attempt: `rtbit-client` on a private current-thread runtime.
#[derive(Clone, Copy, Debug, Default)]
pub struct DaemonLogin;

/// One runtime and one logged-in client, shared by the poll and the row actions.
struct Session {
    runtime: tokio::runtime::Runtime,
    client: Client,
}

fn sessions() -> &'static std::sync::Mutex<Option<Session>> {
    static SESSION: std::sync::OnceLock<std::sync::Mutex<Option<Session>>> =
        std::sync::OnceLock::new();
    SESSION.get_or_init(|| std::sync::Mutex::new(None))
}

impl LoginAttempt for DaemonLogin {
    fn login(&self, url: &str, username: &str, password: &str) -> Result<(), Error> {
        let runtime = runtime()?;
        let client = runtime.block_on(async {
            let mut client = Client::new(url)?;
            client.login(username, password).await?;
            Ok::<Client, Error>(client)
        })?;
        *sessions().lock().unwrap_or_else(|err| err.into_inner()) =
            Some(Session { runtime, client });
        Ok(())
    }

    fn snapshot(&self) -> Result<Snapshot, Error> {
        on_session(|session| {
            session.runtime.block_on(async {
                let torrents = session.client.torrents_with_stats().await?;
                let stats = session.client.stats().await?;
                Ok(Snapshot { torrents, stats })
            })
        })
    }

    fn act(&self, id: u64, action: RowAction) -> Result<(), Error> {
        on_session(|session| {
            session.runtime.block_on(async {
                match action {
                    RowAction::Start => session.client.start_torrent(id).await,
                    RowAction::Pause => session.client.pause_torrent(id).await,
                    RowAction::Delete => session.client.delete_torrent(id).await,
                }
            })
        })
    }

    fn add(&self, request: &AddRequest) -> Result<(), Error> {
        let source = request.source.clone();
        let save_path = request.save_path.clone();
        let category = request.category.clone();
        let start_paused = request.start_paused;
        on_session(move |session| {
            session.runtime.block_on(async {
                let path = Some(save_path.as_str());
                let category = if category.is_empty() {
                    None
                } else {
                    Some(category.as_str())
                };
                match source {
                    AddSource::Magnet(magnet) => {
                        session
                            .client
                            .add_magnet(&magnet, path, category, start_paused)
                            .await
                    }
                    AddSource::File { bytes, .. } => {
                        session
                            .client
                            .add_torrent_file(&bytes, path, category, start_paused)
                            .await
                    }
                }
            })
        })
    }
}

fn on_session<T>(call: impl FnOnce(&mut Session) -> Result<T, Error>) -> Result<T, Error> {
    let mut guard = sessions().lock().unwrap_or_else(|err| err.into_inner());
    let session = guard.as_mut().ok_or(Error::Unauthorised)?;
    call(session)
}

fn runtime() -> Result<tokio::runtime::Runtime, Error> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| Error::Unreachable(format!("could not start the client: {err}")))
}

struct PendingLogin {
    rx: Receiver<Result<(), String>>,
}

/// The shell: chrome, the current screen, and one in-flight login.
pub struct App<L: LoginAttempt = DaemonLogin> {
    screen: Screen,
    connection: Connection,
    url: String,
    username: String,
    password: String,
    /// Shown under the form after a failed attempt. Empty when there is none.
    error: String,
    login: L,
    pending: Option<PendingLogin>,
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

impl<L: LoginAttempt + Clone + 'static> App<L> {
    /// `login` is cloned onto the worker thread, so it must not hold the UI.
    #[must_use]
    pub fn with_login(login: L) -> Self {
        Self {
            screen: Screen::Connect,
            connection: Connection::Disconnected,
            url: "http://127.0.0.1:3030".to_owned(),
            username: String::new(),
            password: String::new(),
            error: String::new(),
            login,
            pending: None,
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
        }
    }

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
        self.pending = Some(PendingLogin { rx });
        std::thread::spawn(move || {
            let outcome = login
                .login(&url, &username, &password)
                .map_err(|err| login_message(&err));
            let _ = tx.send(outcome);
        });
    }

    /// Take a finished login, if the worker has sent one.
    pub fn poll_login(&mut self) {
        let finished = self
            .pending
            .as_ref()
            .and_then(|pending| pending.rx.try_recv().ok());
        if let Some(outcome) = finished {
            self.pending = None;
            match outcome {
                Ok(()) => {
                    self.connection = Connection::Connected;
                    self.screen = Screen::Main;
                    self.error.clear();
                    // The first list is due immediately; later polls wait POLL_EVERY.
                    self.poll_at = None;
                }
                Err(message) => {
                    self.connection = Connection::Error;
                    self.screen = Screen::Connect;
                    self.error = message;
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
        self.poll_login();
        self.poll_list();
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
            |ui| match self.screen {
                Screen::Connect => self.connect_screen(ui),
                Screen::Main => self.main_screen(ui),
            },
        );

        let status = self.connection.label().to_owned();
        StatusBar::new("rtbit_status")
            .left(move |ui| {
                ui.label(status);
            })
            .show(ui);
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
        let chosen = draw_main(
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
        );
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
            Err(message) => self.cache.error = Some(message),
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
                    self.cache.error = Some(message);
                }
            }
        }
    }
}

struct Poll {
    rx: Receiver<Result<Snapshot, String>>,
}

struct ActionFlight {
    rx: Receiver<Result<(), String>>,
    action: RowAction,
}

fn login_message(err: &Error) -> String {
    match err {
        Error::Unreachable(detail) => format!("daemon unreachable: {detail}"),
        Error::Unauthorised => "unauthorised".to_owned(),
        Error::NotSetUp => "daemon is not set up".to_owned(),
        Error::Daemon { status } => format!("daemon error {status}"),
    }
}
