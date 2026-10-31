//! Connect screen: daemon URL, username and masked password, one sign-in call
//! on the bridge, and an error bar when the daemon refuses.
//!
//! The sign-in runs on the bridge's tokio runtime. The UI thread only sets the
//! pending flag and awaits the reply in a gpui task. The shell reacts to the
//! [`ConnectEvent`]s to update the connection badge and the window title.
//!
//! When the daemon has no account yet, the same screen switches to the
//! first-run setup form (`POST /auth/setup`). Once at start-up the screen tries
//! to resume from the refresh token in the keychain. Keychain access runs on a
//! blocking thread inside the bridge, never on the UI thread. Only the refresh
//! token is stored; the password is never stored or logged.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use fluent_layout::{MessageBar, MessageIntent};
use fluent_primitives::{Button, ButtonAppearance, Label, TextInput};
use gpui::{
    AsyncApp, ClickEvent, Context, Entity, EventEmitter, IntoElement, Render, WeakEntity, Window,
    div, prelude::*, px,
};
use rtbit_client::Error;
#[cfg(test)]
use rtbit_ui_core::MemoryStore;
use rtbit_ui_core::{OsKeychain, TokenStore, map_error};

use crate::bridge::Bridge;

/// Address the client starts with.
pub const DEFAULT_URL: &str = "http://127.0.0.1:3030";

/// Shortest password the first-run form accepts.
const MIN_PASSWORD_CHARS: usize = 4;

/// What the shell needs to know about a sign-in attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectEvent {
    /// A sign-in request is in flight.
    Connecting,
    /// The daemon accepted the credentials. The shell opens the main screen.
    Connected,
    /// The daemon has no account yet. The screen shows the setup form.
    SetupRequired,
    /// The attempt failed. The message is shown on this screen.
    Failed,
    /// The user signed out. The shell returns to the connect form.
    SignedOut,
}

/// Which form the screen shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Daemon URL, username and password.
    Connect,
    /// First-run account: setup token, username, password and confirmation.
    Setup,
}

/// What a bridge call reports back to the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reply {
    Connected,
    SetupRequired,
}

/// Where the refresh token is kept. Tests use the in-memory store.
#[derive(Clone)]
enum Keychain {
    Os(OsKeychain),
    #[cfg(test)]
    Memory(MemoryStore),
}

impl Keychain {
    fn load(&self, url: &str) -> Option<Option<String>> {
        match self {
            Self::Os(store) => store.load(url),
            #[cfg(test)]
            Self::Memory(store) => store.load(url),
        }
    }

    fn store(&self, url: &str, token: &str) -> Option<()> {
        match self {
            Self::Os(store) => store.store(url, token),
            #[cfg(test)]
            Self::Memory(store) => store.store(url, token),
        }
    }

    fn clear(&self, url: &str) -> Option<()> {
        match self {
            Self::Os(store) => store.clear(url),
            #[cfg(test)]
            Self::Memory(store) => store.clear(url),
        }
    }
}

/// Read the stored refresh token on a blocking thread. `None` when there is no
/// token or the keychain cannot be opened.
async fn load_token(keychain: Keychain, url: String) -> Option<String> {
    tokio::task::spawn_blocking(move || keychain.load(&url))
        .await
        .ok()
        .flatten()
        .flatten()
}

/// The screen's attempt counter, shared with the bridge so a keychain write can
/// tell whether its attempt is still current. Each sign-in, setup or sign-out
/// bumps it on the UI thread before its own request goes out.
///
/// Every keychain write holds `lock` across the check and the write. A stale
/// resume therefore either sees the bump and skips its write, or finishes its
/// write before the newer attempt's own write runs.
#[derive(Clone, Default)]
struct Epoch {
    current: Arc<AtomicU64>,
    lock: Arc<Mutex<()>>,
}

impl Epoch {
    fn get(&self) -> u64 {
        self.current.load(Ordering::SeqCst)
    }

    fn bump(&self) {
        self.current.fetch_add(1, Ordering::SeqCst);
    }

    /// Run `write` unless `expected` is set and an attempt started after it.
    fn write_if_current(&self, expected: Option<u64>, write: impl FnOnce()) {
        let _guard = self
            .lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if expected.is_none_or(|epoch| epoch == self.get()) {
            write();
        }
    }
}

/// Write the refresh token on a blocking thread. A refused write does not fail
/// the sign-in; the token just will not survive a relaunch. With `expected`
/// set, the write is skipped once a newer attempt has started.
async fn remember_token(
    keychain: Keychain,
    epoch: Epoch,
    expected: Option<u64>,
    url: String,
    token: Option<String>,
) {
    let Some(token) = token else { return };
    let _ = tokio::task::spawn_blocking(move || {
        epoch.write_if_current(expected, || {
            let _ = keychain.store(&url, &token);
        });
    })
    .await;
}

/// Drop the stored refresh token on a blocking thread. Used by sign-out and by
/// a resume the daemon refused, so a dead token is not retried on each launch.
/// With `expected` set, the delete is skipped once a newer attempt has started.
async fn forget_token(keychain: Keychain, epoch: Epoch, expected: Option<u64>, url: String) {
    let _ = tokio::task::spawn_blocking(move || {
        epoch.write_if_current(expected, || {
            let _ = keychain.clear(&url);
        });
    })
    .await;
}

/// The connect and first-run setup form. Fields are read when the user submits;
/// nothing is logged.
pub struct ConnectScreen {
    url: Entity<TextInput>,
    username: Entity<TextInput>,
    password: Entity<TextInput>,
    setup_token: Entity<TextInput>,
    confirm: Entity<TextInput>,
    mode: Mode,
    keychain: Keychain,
    error: Option<String>,
    pending: bool,
    /// A silent start-up resume is in flight. It does not count as `pending`,
    /// so Sign in stays usable while the daemon answers.
    resuming: bool,
    /// Bumped whenever a sign-in or setup starts, so a resume reply that
    /// arrives after that is ignored.
    epoch: Epoch,
}

/// Shown when the daemon has closed first-run setup (403/404 on `/auth/setup`).
const SETUP_CLOSED: &str = "Setup is closed on this daemon. Sign in instead.";

impl EventEmitter<ConnectEvent> for ConnectScreen {}

impl ConnectScreen {
    #[must_use]
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self::build(cx, Keychain::Os(OsKeychain))
    }

    #[cfg(test)]
    fn with_memory_store(cx: &mut Context<Self>, store: MemoryStore) -> Self {
        Self::build(cx, Keychain::Memory(store))
    }

    /// Type the three connect fields, for tests outside this module.
    #[cfg(test)]
    pub(crate) fn fill_for_test(&self, cx: &mut gpui::App, url: &str, user: &str, pass: &str) {
        self.url.update(cx, |field, cx| field.set_value(url, cx));
        self.username
            .update(cx, |field, cx| field.set_value(user, cx));
        self.password
            .update(cx, |field, cx| field.set_value(pass, cx));
    }

    fn build(cx: &mut Context<Self>, keychain: Keychain) -> Self {
        let url = cx.new(|_| TextInput::new().value(DEFAULT_URL));
        let username = cx.new(|_| TextInput::new().placeholder("Username"));
        let password = cx.new(|_| TextInput::new().placeholder("Password").masked(true));
        let setup_token = cx.new(|_| TextInput::new().placeholder("Setup token").masked(true));
        let confirm = cx.new(|_| {
            TextInput::new()
                .placeholder("Confirm password")
                .masked(true)
        });
        Self {
            url,
            username,
            password,
            setup_token,
            confirm,
            mode: Mode::Connect,
            keychain,
            error: None,
            pending: false,
            resuming: false,
            epoch: Epoch::default(),
        }
    }

    /// The message shown in the error bar, if the last attempt failed.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// `true` while a sign-in or setup request is in flight.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// `true` while any request is in flight, including a silent resume.
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.pending || self.resuming
    }

    /// The form currently shown.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Validate the form and start one sign-in on the bridge. The daemon is
    /// asked first whether setup is still open; if it is, the screen switches
    /// to the setup form instead of signing in. A second submit while one is
    /// pending does nothing.
    pub fn submit(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let url = self.url.read(cx).text().trim().to_owned();
        let username = self.username.read(cx).text().trim().to_owned();
        let password = self.password.read(cx).text().to_owned();
        if url.is_empty() || username.is_empty() || password.is_empty() {
            self.fail("Daemon URL, username and password are required.", cx);
            return;
        }
        let bridge = match Bridge::new(&url) {
            Ok(bridge) => bridge,
            Err(err) => {
                self.fail(&map_error(&err), cx);
                return;
            }
        };
        bridge.install(cx);

        self.epoch.bump();
        self.pending = true;
        self.error = None;
        cx.emit(ConnectEvent::Connecting);
        cx.notify();

        let keychain = self.keychain.clone();
        let gate = self.epoch.clone();
        let rx = cx.global::<Bridge>().call(move |client| async move {
            let mut client = client.lock().await;
            if client.auth_status().await?.setup_required {
                return Ok(Reply::SetupRequired);
            }
            client.login(&username, &password).await?;
            let refresh = client.refresh_token().map(str::to_owned);
            remember_token(keychain, gate, None, url, refresh).await;
            Ok(Reply::Connected)
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let reply = reply_or_cancel(rx).await;
            let _ = this.update(cx, |this, cx| this.finish(reply, cx));
        })
        .detach();
    }

    /// Validate the first-run form and create the account on the bridge. The
    /// setup token and passwords are never logged.
    pub fn submit_setup(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let url = self.url.read(cx).text().trim().to_owned();
        let setup_token = self.setup_token.read(cx).text().trim().to_owned();
        let username = self.username.read(cx).text().trim().to_owned();
        let password = self.password.read(cx).text().to_owned();
        let confirm = self.confirm.read(cx).text().to_owned();
        if setup_token.is_empty() || username.is_empty() || password.is_empty() {
            self.fail("Setup token, username and password are required.", cx);
            return;
        }
        if password.chars().count() < MIN_PASSWORD_CHARS {
            self.fail("Password must be at least 4 characters.", cx);
            return;
        }
        if password != confirm {
            self.fail("Passwords do not match.", cx);
            return;
        }
        let bridge = match Bridge::new(&url) {
            Ok(bridge) => bridge,
            Err(err) => {
                self.fail(&map_error(&err), cx);
                return;
            }
        };
        bridge.install(cx);

        self.epoch.bump();
        self.pending = true;
        self.error = None;
        cx.emit(ConnectEvent::Connecting);
        cx.notify();

        let keychain = self.keychain.clone();
        let gate = self.epoch.clone();
        let rx = cx.global::<Bridge>().call(move |client| async move {
            let mut client = client.lock().await;
            client.setup(&username, &password, &setup_token).await?;
            let refresh = client.refresh_token().map(str::to_owned);
            remember_token(keychain, gate, None, url, refresh).await;
            Ok(Reply::Connected)
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let reply = reply_or_cancel(rx).await;
            let _ = this.update(cx, |this, cx| this.finish_setup(reply, cx));
        })
        .detach();
    }

    /// A setup reply. The daemon answers 403/404 once setup has closed, so
    /// that case gets its own sentence instead of the generic not-set-up text.
    fn finish_setup(&mut self, reply: Result<Reply, Error>, cx: &mut Context<Self>) {
        if matches!(reply, Err(Error::NotSetUp)) {
            self.pending = false;
            self.error = Some(SETUP_CLOSED.to_owned());
            cx.emit(ConnectEvent::Failed);
            cx.notify();
            return;
        }
        self.finish(reply, cx);
    }

    /// Try once to continue from the refresh token in the keychain. No token,
    /// an unavailable keychain, or a token the daemon refuses all leave the
    /// connect form as it is, with no error bar.
    pub fn try_resume(&mut self, cx: &mut Context<Self>) {
        if self.pending || self.resuming || self.mode != Mode::Connect {
            return;
        }
        let url = self.url.read(cx).text().trim().to_owned();
        if url.is_empty() {
            return;
        }
        let Ok(bridge) = Bridge::new(&url) else {
            return;
        };
        bridge.install(cx);

        self.resuming = true;
        let epoch = self.epoch.get();
        cx.notify();

        let keychain = self.keychain.clone();
        let gate = self.epoch.clone();
        let rx = cx.global::<Bridge>().call(move |client| async move {
            let Some(refresh) = load_token(keychain.clone(), url.clone()).await else {
                return Err(Error::Unauthorised);
            };
            let mut client = client.lock().await;
            match client.resume(&refresh).await {
                Ok(()) => {
                    let rotated = client.refresh_token().map(str::to_owned);
                    remember_token(keychain, gate, Some(epoch), url, rotated).await;
                    Ok(Reply::Connected)
                }
                Err(err) => {
                    // The daemon no longer accepts the stored token: forget it,
                    // so each launch does not retry a dead token. Not if a newer
                    // sign-in has started: the stored token may be its own.
                    if matches!(err, Error::Unauthorised) {
                        forget_token(keychain, gate, Some(epoch), url).await;
                    }
                    Err(err)
                }
            }
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let reply = reply_or_cancel(rx).await;
            let _ = this.update(cx, |this, cx| this.finish_resume(epoch, reply, cx));
        })
        .detach();
    }

    fn finish(&mut self, reply: Result<Reply, Error>, cx: &mut Context<Self>) {
        self.pending = false;
        match reply {
            Ok(Reply::Connected) => {
                self.clear_secrets(cx);
                cx.emit(ConnectEvent::Connected);
                cx.notify();
            }
            Ok(Reply::SetupRequired) => {
                // The typed password was for a sign-in that cannot happen yet.
                self.password
                    .update(cx, |field, cx| field.set_value("", cx));
                self.mode = Mode::Setup;
                self.error = None;
                cx.emit(ConnectEvent::SetupRequired);
                cx.notify();
            }
            Err(err) => self.fail(&map_error(&err), cx),
        }
    }

    /// A resume is silent: success opens the main screen, anything else leaves
    /// the form as it was.
    fn finish_resume(&mut self, epoch: u64, reply: Result<Reply, Error>, cx: &mut Context<Self>) {
        self.resuming = false;
        // A sign-in started after this resume: its own reply decides the screen.
        if epoch != self.epoch.get() {
            cx.notify();
            return;
        }
        if matches!(reply, Ok(Reply::Connected)) {
            self.clear_secrets(cx);
            cx.emit(ConnectEvent::Connected);
        }
        cx.notify();
    }

    fn clear_secrets(&self, cx: &mut Context<Self>) {
        for field in [&self.password, &self.setup_token, &self.confirm] {
            field.update(cx, |field, cx| field.set_value("", cx));
        }
    }

    /// Sign out: drop the live daemon session and the stored refresh token on
    /// the bridge, clear the secret fields, and return to the connect form.
    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        // A resume still in flight must not write its token back after this.
        self.epoch.bump();
        let url = self.url.read(cx).text().trim().to_owned();
        if let Ok(bridge) = Bridge::new(&url) {
            bridge.install(cx);
            let keychain = self.keychain.clone();
            let gate = self.epoch.clone();
            let rx = cx.global::<Bridge>().call(move |client| async move {
                let mut client = client.lock().await;
                client.logout();
                forget_token(keychain, gate, None, url).await;
                Ok::<(), Error>(())
            });
            cx.spawn(async move |_this: WeakEntity<Self>, _cx: &mut AsyncApp| {
                let _ = rx.await;
            })
            .detach();
        }
        self.mode = Mode::Connect;
        self.error = None;
        self.clear_secrets(cx);
        cx.emit(ConnectEvent::SignedOut);
        cx.notify();
    }

    fn fail(&mut self, message: &str, cx: &mut Context<Self>) {
        self.pending = false;
        self.error = Some(message.to_owned());
        cx.emit(ConnectEvent::Failed);
        cx.notify();
    }

    /// Leave the first-run setup form for the sign-in form. The daemon URL
    /// stays as typed; the setup secrets are cleared, and any setup error goes
    /// with them. Ignored while a request is in flight.
    pub fn back_to_connect(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        self.mode = Mode::Connect;
        self.error = None;
        self.clear_secrets(cx);
        cx.notify();
    }
}

/// The reply from a bridge call, or a cancellation error if the runtime dropped it.
async fn reply_or_cancel(
    rx: tokio::sync::oneshot::Receiver<Result<Reply, Error>>,
) -> Result<Reply, Error> {
    match rx.await {
        Ok(reply) => reply,
        Err(_) => Err(Error::Unreachable("request cancelled".to_owned())),
    }
}

fn field(label: &'static str, input: Entity<TextInput>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(Label::new(label))
        .child(input)
}

impl Render for ConnectScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut form = div().flex().flex_col().gap_3().w(px(360.0));
        match self.mode {
            Mode::Connect => {
                let submit = cx.listener(|this, _: &ClickEvent, _window, cx| this.submit(cx));
                let label = if self.pending {
                    "Connecting…"
                } else {
                    "Sign in"
                };
                form = form
                    .child(Label::new("Connect to a daemon"))
                    .child(field("Daemon URL", self.url.clone()))
                    .child(field("Username", self.username.clone()))
                    .child(field("Password", self.password.clone()));
                if let Some(message) = &self.error {
                    form =
                        form.child(MessageBar::new(message.clone()).intent(MessageIntent::Error));
                }
                form.child(
                    Button::new("connect-submit")
                        .label(label)
                        .appearance(ButtonAppearance::Accent)
                        .disabled(self.pending)
                        .on_click(submit),
                )
            }
            Mode::Setup => {
                let submit = cx.listener(|this, _: &ClickEvent, _window, cx| this.submit_setup(cx));
                let back =
                    cx.listener(|this, _: &ClickEvent, _window, cx| this.back_to_connect(cx));
                let label = if self.pending {
                    "Creating account…"
                } else {
                    "Create account"
                };
                form = form
                    .child(Label::new("First-run setup"))
                    .child(Label::new("The setup token is printed in the rtbit log."))
                    .child(field("Daemon URL", self.url.clone()))
                    .child(field("Setup token", self.setup_token.clone()))
                    .child(field("Username", self.username.clone()))
                    .child(field("Password", self.password.clone()))
                    .child(field("Confirm password", self.confirm.clone()));
                if let Some(message) = &self.error {
                    form =
                        form.child(MessageBar::new(message.clone()).intent(MessageIntent::Error));
                }
                form.child(
                    Button::new("setup-submit")
                        .label(label)
                        .appearance(ButtonAppearance::Accent)
                        .disabled(self.pending)
                        .on_click(submit),
                )
                .child(
                    Button::new("setup-back")
                        .label("Back")
                        .appearance(ButtonAppearance::Hyperlink)
                        .disabled(self.pending)
                        .on_click(back),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use gpui::{App, Subscription, TestAppContext};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../rtbit-client/tests/fixtures"
    );

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap()
    }

    /// A wiremock daemon. `/auth/status` says whether setup is still open;
    /// `/auth/login` answers `login`; `/auth/setup` and `/auth/refresh` succeed
    /// with the token fixtures.
    fn daemon(runtime: &tokio::runtime::Runtime, login: u16, setup_required: bool) -> MockServer {
        runtime.block_on(async {
            let server = MockServer::start().await;
            let status_body =
                format!(r#"{{"auth_enabled":true,"setup_required":{setup_required}}}"#);
            Mock::given(method("GET"))
                .and(path("/auth/status"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_raw(status_body, "application/json"),
                )
                .mount(&server)
                .await;
            let login_response = if login == 200 {
                ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json")
            } else {
                ResponseTemplate::new(login)
            };
            Mock::given(method("POST"))
                .and(path("/auth/login"))
                .respond_with(login_response)
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/auth/setup"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(fixture("login.json"), "application/json"),
                )
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/auth/refresh"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(fixture("refresh.json"), "application/json"),
                )
                .mount(&server)
                .await;
            server
        })
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap()
    }

    /// Put text into the three connect fields the way a user would type it.
    fn fill(
        screen: &Entity<ConnectScreen>,
        cx: &mut TestAppContext,
        url: &str,
        user: &str,
        pass: &str,
    ) {
        let (url_field, user_field, pass_field) = screen.read_with(cx, |s, _| {
            (s.url.clone(), s.username.clone(), s.password.clone())
        });
        url_field.update(cx, |field, cx| field.set_value(url, cx));
        user_field.update(cx, |field, cx| field.set_value(user, cx));
        pass_field.update(cx, |field, cx| field.set_value(pass, cx));
    }

    /// Put text into the four first-run fields.
    fn fill_setup(
        screen: &Entity<ConnectScreen>,
        cx: &mut TestAppContext,
        token: &str,
        user: &str,
        pass: &str,
        confirm: &str,
    ) {
        let (token_field, user_field, pass_field, confirm_field) = screen.read_with(cx, |s, _| {
            (
                s.setup_token.clone(),
                s.username.clone(),
                s.password.clone(),
                s.confirm.clone(),
            )
        });
        token_field.update(cx, |field, cx| field.set_value(token, cx));
        user_field.update(cx, |field, cx| field.set_value(user, cx));
        pass_field.update(cx, |field, cx| field.set_value(pass, cx));
        confirm_field.update(cx, |field, cx| field.set_value(confirm, cx));
    }

    /// Drive gpui until `done` holds. Replies arrive from tokio threads.
    fn drive(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
        for _ in 0..500 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("sign-in reply never arrived");
    }

    fn record_events(
        screen: &Entity<ConnectScreen>,
        cx: &mut TestAppContext,
    ) -> (Rc<RefCell<Vec<ConnectEvent>>>, Subscription) {
        let events: Rc<RefCell<Vec<ConnectEvent>>> = Rc::default();
        let sink = events.clone();
        let subscription = cx.update(|cx: &mut App| {
            cx.subscribe(screen, move |_, event: &ConnectEvent, _| {
                sink.borrow_mut().push(*event);
            })
        });
        (events, subscription)
    }

    #[gpui::test]
    fn empty_fields_show_an_error_without_a_request(cx: &mut TestAppContext) {
        let screen = cx.new(ConnectScreen::new);
        screen.update(cx, |screen, cx| screen.submit(cx));
        screen.read_with(cx, |screen, _| {
            assert!(!screen.is_pending());
            assert_eq!(
                screen.error(),
                Some("Daemon URL, username and password are required.")
            );
        });
    }

    #[gpui::test]
    fn bad_credentials_show_the_error_bar(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = runtime();
        let server = daemon(&runtime, 401, false);
        let screen = cx.new(ConnectScreen::new);
        fill(&screen, cx, &server.uri(), "admin", "wrong");
        screen.update(cx, |screen, cx| screen.submit(cx));
        assert!(screen.read_with(cx, |s, _| s.is_pending()));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_pending()));
        screen.read_with(cx, |screen, _| {
            assert_eq!(screen.error(), Some("Sign-in was refused."));
            assert_eq!(screen.mode(), Mode::Connect);
        });
    }

    #[gpui::test]
    fn good_credentials_emit_connected_and_clear_the_password(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = runtime();
        let server = daemon(&runtime, 200, false);
        let screen = cx.new(ConnectScreen::new);
        let (events, _subscription) = record_events(&screen, cx);
        fill(&screen, cx, &server.uri(), "admin", "secret");
        screen.update(cx, |screen, cx| screen.submit(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_pending()));
        assert_eq!(
            *events.borrow(),
            vec![ConnectEvent::Connecting, ConnectEvent::Connected]
        );
        screen.read_with(cx, |screen, _| assert!(screen.error().is_none()));
        let pass = screen.read_with(cx, |s, _| s.password.clone());
        assert_eq!(pass.read_with(cx, |field, _| field.text().to_owned()), "");
    }

    #[gpui::test]
    fn open_setup_switches_to_the_setup_form_without_signing_in(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = runtime();
        let server = daemon(&runtime, 200, true);
        let screen = cx.new(ConnectScreen::new);
        let (events, _subscription) = record_events(&screen, cx);
        fill(&screen, cx, &server.uri(), "admin", "secret");
        screen.update(cx, |screen, cx| screen.submit(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_pending()));
        assert_eq!(
            *events.borrow(),
            vec![ConnectEvent::Connecting, ConnectEvent::SetupRequired]
        );
        screen.read_with(cx, |screen, _| {
            assert_eq!(screen.mode(), Mode::Setup);
            assert!(screen.error().is_none());
        });
    }

    #[gpui::test]
    fn setup_creates_the_account_and_stores_only_the_refresh_token(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = runtime();
        let server = daemon(&runtime, 200, true);
        let store = MemoryStore::default();
        let screen = cx.new(|cx| ConnectScreen::with_memory_store(cx, store.clone()));
        let (events, _subscription) = record_events(&screen, cx);
        fill(&screen, cx, &server.uri(), "admin", "secret");
        screen.update(cx, |screen, cx| screen.submit(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_pending()));
        fill_setup(&screen, cx, "setup-token", "admin", "secret", "secret");
        screen.update(cx, |screen, cx| screen.submit_setup(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_pending()));
        assert_eq!(
            *events.borrow(),
            vec![
                ConnectEvent::Connecting,
                ConnectEvent::SetupRequired,
                ConnectEvent::Connecting,
                ConnectEvent::Connected,
            ]
        );
        assert_eq!(
            store.load(&server.uri()),
            Some(Some("refresh-1".to_owned()))
        );
        let setup_token = screen.read_with(cx, |s, _| s.setup_token.clone());
        assert_eq!(setup_token.read_with(cx, |f, _| f.text().to_owned()), "");
    }

    #[gpui::test]
    fn setup_rejects_a_short_password_without_a_request(cx: &mut TestAppContext) {
        let screen = cx.new(ConnectScreen::new);
        screen.update(cx, |screen, _| screen.mode = Mode::Setup);
        fill_setup(&screen, cx, "setup-token", "admin", "abc", "abc");
        screen.update(cx, |screen, cx| screen.submit_setup(cx));
        screen.read_with(cx, |screen, _| {
            assert!(!screen.is_pending());
            assert_eq!(
                screen.error(),
                Some("Password must be at least 4 characters.")
            );
        });
    }

    #[gpui::test]
    fn back_from_setup_returns_to_connect_and_clears_setup_secrets(cx: &mut TestAppContext) {
        let screen = cx.new(ConnectScreen::new);
        screen.update(cx, |screen, _| screen.mode = Mode::Setup);
        fill_setup(&screen, cx, "setup-token", "admin", "abc", "abc");
        screen.update(cx, |screen, cx| screen.submit_setup(cx));
        screen.read_with(cx, |screen, _| assert!(screen.error().is_some()));
        screen.update(cx, |screen, cx| screen.back_to_connect(cx));
        screen.read_with(cx, |screen, cx| {
            assert_eq!(screen.mode(), Mode::Connect);
            assert!(screen.error().is_none(), "setup error goes with the form");
            assert_eq!(screen.setup_token.read(cx).text(), "");
            assert_eq!(screen.confirm.read(cx).text(), "");
            assert_eq!(screen.password.read(cx).text(), "");
            assert_eq!(screen.username.read(cx).text(), "admin");
        });
    }

    #[gpui::test]
    fn setup_form_shows_the_daemon_url_the_user_typed(cx: &mut TestAppContext) {
        let screen = cx.new(ConnectScreen::new);
        fill(&screen, cx, "http://10.0.0.5:3030", "admin", "secret");
        screen.update(cx, |screen, _| screen.mode = Mode::Setup);
        screen.read_with(cx, |screen, cx| {
            assert_eq!(screen.url.read(cx).text(), "http://10.0.0.5:3030");
        });
    }

    #[gpui::test]
    fn setup_rejects_mismatched_passwords_without_a_request(cx: &mut TestAppContext) {
        let screen = cx.new(ConnectScreen::new);
        screen.update(cx, |screen, _| screen.mode = Mode::Setup);
        fill_setup(&screen, cx, "setup-token", "admin", "secret", "other");
        screen.update(cx, |screen, cx| screen.submit_setup(cx));
        screen.read_with(cx, |screen, _| {
            assert!(!screen.is_pending());
            assert_eq!(screen.error(), Some("Passwords do not match."));
        });
    }

    #[gpui::test]
    fn stored_refresh_token_resumes_and_is_rotated(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = runtime();
        let server = daemon(&runtime, 200, false);
        let store = MemoryStore::default();
        store.store(&server.uri(), "refresh-1");
        let screen = cx.new(|cx| ConnectScreen::with_memory_store(cx, store.clone()));
        let (events, _subscription) = record_events(&screen, cx);
        let url = screen.read_with(cx, |s, _| s.url.clone());
        url.update(cx, |field, cx| field.set_value(server.uri(), cx));
        screen.update(cx, |screen, cx| screen.try_resume(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_busy()));
        assert_eq!(*events.borrow(), vec![ConnectEvent::Connected]);
        screen.read_with(cx, |screen, _| assert!(screen.error().is_none()));
        assert_eq!(
            store.load(&server.uri()),
            Some(Some("refresh-2".to_owned()))
        );
    }

    #[gpui::test]
    fn no_stored_token_resumes_silently(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = runtime();
        let server = daemon(&runtime, 200, false);
        let store = MemoryStore::default();
        let screen = cx.new(|cx| ConnectScreen::with_memory_store(cx, store.clone()));
        let (events, _subscription) = record_events(&screen, cx);
        let url = screen.read_with(cx, |s, _| s.url.clone());
        url.update(cx, |field, cx| field.set_value(server.uri(), cx));
        screen.update(cx, |screen, cx| screen.try_resume(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_busy()));
        assert!(events.borrow().is_empty());
        screen.read_with(cx, |screen, _| assert!(screen.error().is_none()));
    }

    #[gpui::test]
    fn sign_out_clears_the_stored_token_and_returns_to_connect(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = runtime();
        let server = daemon(&runtime, 200, false);
        let store = MemoryStore::default();
        store.store(&server.uri(), "refresh-1");
        let screen = cx.new(|cx| ConnectScreen::with_memory_store(cx, store.clone()));
        let (events, _subscription) = record_events(&screen, cx);
        let url = screen.read_with(cx, |s, _| s.url.clone());
        url.update(cx, |field, cx| field.set_value(server.uri(), cx));
        screen.update(cx, |screen, cx| screen.try_resume(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_busy()));
        assert_eq!(*events.borrow(), vec![ConnectEvent::Connected]);

        screen.update(cx, |screen, cx| screen.sign_out(cx));
        // The clear runs on the bridge, so wait for it rather than assert early.
        drive(cx, |_| store.load(&server.uri()) == Some(None));
        assert_eq!(
            *events.borrow(),
            vec![ConnectEvent::Connected, ConnectEvent::SignedOut]
        );
        screen.read_with(cx, |screen, _| {
            assert_eq!(screen.mode(), Mode::Connect);
            assert!(screen.error().is_none());
        });
    }

    #[gpui::test]
    fn a_refused_stored_token_is_cleared_from_the_keychain(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = runtime();
        // A daemon that refuses every refresh: the stored token is dead.
        let server = runtime.block_on(async {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/auth/status"))
                .respond_with(ResponseTemplate::new(200).set_body_raw(
                    r#"{"auth_enabled":true,"setup_required":false}"#,
                    "application/json",
                ))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/auth/refresh"))
                .respond_with(ResponseTemplate::new(401))
                .mount(&server)
                .await;
            server
        });
        let store = MemoryStore::default();
        store.store(&server.uri(), "dead-token");
        let screen = cx.new(|cx| ConnectScreen::with_memory_store(cx, store.clone()));
        let (events, _subscription) = record_events(&screen, cx);
        let url = screen.read_with(cx, |s, _| s.url.clone());
        url.update(cx, |field, cx| field.set_value(server.uri(), cx));
        screen.update(cx, |screen, cx| screen.try_resume(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_busy()));
        assert!(events.borrow().is_empty(), "a refused resume stays silent");
        // The forget runs on the bridge; wait for it.
        drive(cx, |_| store.load(&server.uri()) == Some(None));
    }

    #[gpui::test]
    fn sign_in_stays_enabled_while_a_stored_token_resume_is_in_flight(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = runtime();
        // The refresh answers slowly, so the resume is still in flight for a while.
        let server = runtime.block_on(async {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/auth/status"))
                .respond_with(ResponseTemplate::new(200).set_body_raw(
                    r#"{"auth_enabled":true,"setup_required":false}"#,
                    "application/json",
                ))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/auth/refresh"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(fixture("refresh.json"), "application/json")
                        .set_delay(std::time::Duration::from_millis(1500)),
                )
                .mount(&server)
                .await;
            server
        });
        let store = MemoryStore::default();
        store.store(&server.uri(), "refresh-1");
        let screen = cx.new(|cx| ConnectScreen::with_memory_store(cx, store.clone()));
        let url = screen.read_with(cx, |s, _| s.url.clone());
        url.update(cx, |field, cx| field.set_value(server.uri(), cx));
        screen.update(cx, |screen, cx| screen.try_resume(cx));
        // While the resume is in flight, Sign in is not locked: `pending` stays false.
        screen.read_with(cx, |s, _| {
            assert!(s.is_busy());
            assert!(!s.is_pending(), "a silent resume must not lock Sign in");
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_busy()));
    }

    /// A daemon whose `/auth/refresh` answers `status` after `delay_ms`, and
    /// whose sign-in succeeds at once with the login fixture.
    fn slow_refresh_daemon(
        runtime: &tokio::runtime::Runtime,
        status: u16,
        delay_ms: u64,
    ) -> MockServer {
        runtime.block_on(async {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/auth/status"))
                .respond_with(ResponseTemplate::new(200).set_body_raw(
                    r#"{"auth_enabled":true,"setup_required":false}"#,
                    "application/json",
                ))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/auth/login"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(fixture("login.json"), "application/json"),
                )
                .mount(&server)
                .await;
            let refresh = if status == 200 {
                ResponseTemplate::new(200).set_body_raw(fixture("refresh.json"), "application/json")
            } else {
                ResponseTemplate::new(status)
            };
            Mock::given(method("POST"))
                .and(path("/auth/refresh"))
                .respond_with(refresh.set_delay(Duration::from_millis(delay_ms)))
                .mount(&server)
                .await;
            server
        })
    }

    #[gpui::test]
    fn a_resume_finishing_after_sign_out_does_not_store_its_token(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = runtime();
        let server = slow_refresh_daemon(&runtime, 200, 500);
        let store = MemoryStore::default();
        store.store(&server.uri(), "refresh-1");
        let screen = cx.new(|cx| ConnectScreen::with_memory_store(cx, store.clone()));
        let url = screen.read_with(cx, |s, _| s.url.clone());
        url.update(cx, |field, cx| field.set_value(server.uri(), cx));
        screen.update(cx, |screen, cx| screen.try_resume(cx));
        // Let the resume read the keychain and reach the slow refresh first.
        std::thread::sleep(Duration::from_millis(150));
        screen.update(cx, |screen, cx| screen.sign_out(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_busy()));
        // Give the sign-out's own clear time to land as well.
        std::thread::sleep(Duration::from_millis(200));
        cx.run_until_parked();
        assert_eq!(store.load(&server.uri()), Some(None));
    }

    #[gpui::test]
    fn a_stale_refused_resume_keeps_the_newer_sign_in_token(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = runtime();
        let server = slow_refresh_daemon(&runtime, 401, 500);
        let store = MemoryStore::default();
        store.store(&server.uri(), "dead-token");
        let screen = cx.new(|cx| ConnectScreen::with_memory_store(cx, store.clone()));
        fill(&screen, cx, &server.uri(), "admin", "secret");
        screen.update(cx, |screen, cx| screen.try_resume(cx));
        std::thread::sleep(Duration::from_millis(150));
        screen.update(cx, |screen, cx| screen.submit(cx));
        drive(cx, |cx| {
            screen.read_with(cx, |s, _| !s.is_busy() && !s.is_pending())
        });
        // Outlast the slow refusal so the stale resume has had its chance to forget.
        std::thread::sleep(Duration::from_millis(700));
        cx.run_until_parked();
        assert_eq!(
            store.load(&server.uri()),
            Some(Some("refresh-1".to_owned()))
        );
    }

    #[gpui::test]
    fn setup_closed_on_the_daemon_shows_its_own_message(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = runtime();
        let server = runtime.block_on(async {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/auth/status"))
                .respond_with(ResponseTemplate::new(200).set_body_raw(
                    r#"{"auth_enabled":true,"setup_required":true}"#,
                    "application/json",
                ))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/auth/setup"))
                .respond_with(ResponseTemplate::new(403))
                .mount(&server)
                .await;
            server
        });
        let screen = cx.new(ConnectScreen::new);
        // Open the setup form the way a real sign-in does, when the daemon reports setup_required.
        fill(&screen, cx, &server.uri(), "admin", "secret");
        screen.update(cx, |screen, cx| screen.submit(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_pending()));
        screen.read_with(cx, |s, _| assert_eq!(s.mode(), Mode::Setup));
        fill_setup(&screen, cx, "setup-token", "admin", "secret", "secret");
        screen.update(cx, |screen, cx| screen.submit_setup(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_pending()));
        screen.read_with(cx, |screen, _| {
            assert_eq!(screen.mode(), Mode::Setup);
            assert_eq!(screen.error(), Some(SETUP_CLOSED));
        });
    }

    #[gpui::test]
    fn an_unreachable_daemon_shows_the_unreachable_message(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        // Bind a port, then drop the listener so nothing answers on it.
        let url = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}", listener.local_addr().unwrap())
        };
        let screen = cx.new(ConnectScreen::new);
        fill(&screen, cx, &url, "admin", "secret");
        screen.update(cx, |screen, cx| screen.submit(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_pending()));
        screen.read_with(cx, |screen, _| {
            assert_eq!(screen.error(), Some("The daemon is unreachable."));
        });
    }
}
