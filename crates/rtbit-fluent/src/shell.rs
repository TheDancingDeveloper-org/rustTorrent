//! Window shell: navigation sidebar, screen routing and status bar, inside a `Workspace`.

use fluent_app::WindowTitle;
use fluent_layout::{SelectableList, SelectableListItem};
use gpui::{
    AnyElement, ClipboardItem, Context, Entity, ExternalPaths, IntoElement, Render, Subscription,
    Window, div, prelude::*, px,
};
use rtbit_ui_core::{Connection, Screen, Snapshot, StatusFilter};

use crate::screens::MainScreen;
use crate::screens::connect::{ConnectEvent, ConnectScreen};
use crate::screens::main_screen::MainEvent;
use crate::screens::rss::RssScreen;
use crate::screens::settings::SettingsScreen;
use crate::widgets::status_bar::StatusBar;
use crate::window_state::WindowState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DropKind {
    TorrentFiles,
    Magnet,
    Other,
}

pub(crate) fn classify_drop(paths: Option<&ExternalPaths>, text: Option<&str>) -> DropKind {
    if paths.is_some_and(|paths| {
        !paths.paths().is_empty()
            && paths.paths().iter().all(|path| {
                path.extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent"))
            })
    }) {
        return DropKind::TorrentFiles;
    }
    if text.is_some_and(|text| text.trim_start().starts_with("magnet:?")) {
        return DropKind::Magnet;
    }
    DropKind::Other
}

/// Screens reachable from the sidebar. Connect and Setup come before login and are not listed.
const NAV: [(&str, &str, Screen); 3] = [
    ("torrents", "Torrents", Screen::Main),
    ("rss", "RSS", Screen::Rss),
    ("settings", "Settings", Screen::Settings),
];

/// Routes the active screen and owns the sidebar state.
pub struct Shell {
    screen: Screen,
    filter: StatusFilter,
    status: Entity<StatusBar>,
    connect: Entity<ConnectScreen>,
    rss: Entity<RssScreen>,
    settings: Entity<SettingsScreen>,
    main: Entity<MainScreen>,
    /// Seconds until the next retry while the daemon is lost; `None` when reachable.
    lost_retry_secs: Option<u64>,
    _connect_events: Subscription,
    _main_events: Subscription,
    /// Screen to open on after the next successful sign-in, from the last session.
    restore_screen: Option<Screen>,
    /// Where the current screen is remembered; `None` keeps nothing (tests).
    state_path: Option<std::path::PathBuf>,
}

impl Shell {
    #[must_use]
    pub fn new(
        status: Entity<StatusBar>,
        main: Entity<MainScreen>,
        cx: &mut Context<Self>,
    ) -> Self {
        let connect = cx.new(ConnectScreen::new);
        let settings = cx.new(SettingsScreen::new);
        let connect_events = cx.subscribe(
            &connect,
            |shell: &mut Self,
             _: Entity<ConnectScreen>,
             event: &ConnectEvent,
             cx: &mut Context<Self>| {
                shell.on_connect_event(*event, cx);
            },
        );
        let main_events = cx.subscribe(
            &main,
            |shell: &mut Self, _: Entity<MainScreen>, event: &MainEvent, cx: &mut Context<Self>| {
                shell.on_main_event(*event, cx);
            },
        );
        Self {
            screen: Screen::Connect,
            filter: StatusFilter::All,
            status,
            connect,
            rss: cx.new(RssScreen::new),
            settings,
            main,
            lost_retry_secs: None,
            _connect_events: connect_events,
            _main_events: main_events,
            restore_screen: None,
            state_path: None,
        }
    }

    /// Remember the screen in `path`, and open on `restore` after the first sign-in.
    pub fn remember_screen(&mut self, path: Option<std::path::PathBuf>, restore: Option<Screen>) {
        self.state_path = path;
        self.restore_screen = restore;
    }

    /// Show a screen reached by navigation or restore, loading what it needs.
    fn show(&mut self, screen: Screen, cx: &mut Context<Self>) {
        self.screen = screen;
        if screen == Screen::Rss {
            self.rss.update(cx, |rss, cx| rss.load(cx));
        }
        if screen == Screen::Settings {
            self.settings.update(cx, |settings, cx| settings.load(cx));
        }
        if let Some(path) = &self.state_path {
            let mut state = WindowState::load(path);
            state.screen = Some(screen);
            let _ = state.save(path);
        }
    }

    /// Follow daemon reachability from the main screen's poll.
    fn on_main_event(&mut self, event: MainEvent, cx: &mut Context<Self>) {
        match event {
            MainEvent::DaemonLost { retry_in_secs } => {
                self.lost_retry_secs = Some(retry_in_secs);
                self.set_connection(Connection::Error, cx);
            }
            MainEvent::DaemonRecovered => {
                self.lost_retry_secs = None;
                self.set_connection(Connection::Connected, cx);
            }
            MainEvent::Unauthorised => {
                self.lost_retry_secs = None;
                self.screen = Screen::Connect;
                self.set_connection(Connection::Disconnected, cx);
            }
        }
        cx.notify();
    }

    /// The lost-connection banner text, while the daemon is unreachable.
    #[must_use]
    pub fn lost_banner(&self) -> Option<String> {
        self.lost_retry_secs
            .map(|secs| format!("Lost connection to daemon — retrying in {secs}s"))
    }

    /// Follow a sign-in attempt: badge and title always; the main screen on success.
    fn on_connect_event(&mut self, event: ConnectEvent, cx: &mut Context<Self>) {
        self.lost_retry_secs = None;
        match event {
            ConnectEvent::Connecting => self.set_connection(Connection::Connecting, cx),
            ConnectEvent::Connected => {
                let screen = self.restore_screen.take().unwrap_or(Screen::Main);
                self.show(screen, cx);
                self.set_connection(Connection::Connected, cx);
                self.status.update(cx, |bar, cx| bar.load_settings(cx));
            }
            ConnectEvent::SetupRequired => {
                self.screen = Screen::Setup;
                self.set_connection(Connection::Disconnected, cx);
            }
            ConnectEvent::Failed => self.set_connection(Connection::Error, cx),
            ConnectEvent::SignedOut => {
                self.screen = Screen::Connect;
                self.set_connection(Connection::Disconnected, cx);
            }
        }
        cx.notify();
    }

    /// The screen currently shown.
    #[must_use]
    pub fn screen(&self) -> Screen {
        self.screen
    }

    /// Switch screens by sidebar id. Unknown ids are ignored.
    pub fn select_screen(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some((_, _, screen)) = NAV.iter().find(|(nav_id, _, _)| *nav_id == id) {
            self.show(*screen, cx);
            cx.notify();
        }
    }

    /// Try once to continue from the stored refresh token. Silent when there is none.
    pub fn try_resume(&mut self, cx: &mut Context<Self>) {
        self.connect
            .update(cx, |connect, cx| connect.try_resume(cx));
    }

    /// Set the sidebar status filter by its label. Unknown labels are ignored.
    pub fn select_filter(&mut self, label: &str, cx: &mut Context<Self>) {
        if let Some(filter) = StatusFilter::ALL.iter().find(|f| f.label() == label) {
            self.filter = *filter;
            let filter = *filter;
            self.main.update(cx, |main, cx| main.set_filter(filter, cx));
            cx.notify();
        }
    }

    /// Show the daemon connection in the status bar and the window title.
    pub fn set_connection(&mut self, connection: Connection, cx: &mut Context<Self>) {
        self.status
            .update(cx, |bar, cx| bar.set_connection(connection, cx));
        WindowTitle::set_status(cx, connection.label());
    }

    /// Feed a poll's torrent total and session speeds to the status bar.
    pub fn apply_snapshot(&mut self, snapshot: &Snapshot, cx: &mut Context<Self>) {
        self.status
            .update(cx, |bar, cx| bar.apply_snapshot(snapshot, cx));
    }

    fn body(&self) -> AnyElement {
        if matches!(self.screen, Screen::Connect | Screen::Setup) {
            return div()
                .flex_1()
                .p_4()
                .child(self.connect.clone())
                .into_any_element();
        }
        if self.screen == Screen::Rss {
            return div()
                .flex_1()
                .p_4()
                .child(self.rss.clone())
                .into_any_element();
        }
        if self.screen == Screen::Settings {
            return div()
                .flex_1()
                .p_4()
                .child(self.settings.clone())
                .into_any_element();
        }
        let text = match self.screen {
            Screen::Connect | Screen::Setup => unreachable!("handled above"),
            Screen::Main => return self.main.clone().into_any_element(),
            Screen::Settings => "Settings".to_owned(),
            Screen::Rss => "RSS".to_owned(),
        };
        div()
            .flex_1()
            .p_4()
            .child(fluent_primitives::Label::new(text))
            .into_any_element()
    }
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let me = cx.entity();
        let screen_id = NAV
            .iter()
            .find(|(_, _, screen)| *screen == self.screen)
            .map_or("", |(id, _, _)| *id);

        let nav_items = NAV
            .iter()
            .map(|(id, title, _)| SelectableListItem::new(*id, *title));
        let nav_me = me.clone();
        let nav = SelectableList::new("nav")
            .items(nav_items)
            .selected(screen_id)
            .on_select(move |id, _, cx| {
                nav_me.update(cx, |shell, cx| shell.select_screen(&id, cx));
            });

        let filter_items = StatusFilter::ALL
            .iter()
            .map(|f| SelectableListItem::new(f.label(), f.label()));
        let filter_me = me;
        let filters = SelectableList::new("filters")
            .items(filter_items)
            .selected(self.filter.label())
            .on_select(move |label, _, cx| {
                filter_me.update(cx, |shell, cx| shell.select_filter(&label, cx));
            });

        div()
            .on_drop(cx.listener(|shell, item: &ExternalPaths, window, cx| {
                if classify_drop(Some(item), None) == DropKind::TorrentFiles {
                    shell
                        .main
                        .update(cx, |main, cx| main.open_add_with_paths(item.paths(), cx));
                } else {
                    shell
                        .main
                        .update(cx, |main, cx| main.show_not_torrent_toast(cx));
                }
                window.refresh();
            }))
            .on_drop(cx.listener(|shell, item: &ClipboardItem, window, cx| {
                let text = item.text();
                match classify_drop(None, text.as_deref()) {
                    DropKind::Magnet => shell.main.update(cx, |main, cx| {
                        main.open_add_with_magnet(text.as_deref().unwrap_or_default(), cx);
                    }),
                    DropKind::Other | DropKind::TorrentFiles => shell
                        .main
                        .update(cx, |main, cx| main.show_not_torrent_toast(cx)),
                }
                window.refresh();
            }))
            .flex()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(200.0))
                    .child(nav)
                    .child(filters),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .children(self.lost_banner().map(|text| {
                        div()
                            .id("daemon-lost-banner")
                            .px_3()
                            .py_1()
                            .child(fluent_primitives::Label::new(text))
                    }))
                    .child(self.body()),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gpui::TestAppContext;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[test]
    fn classifies_torrent_magnet_and_other_drops() {
        let paths = ExternalPaths(
            [std::path::PathBuf::from("one.TORRENT")]
                .into_iter()
                .collect(),
        );
        assert_eq!(classify_drop(Some(&paths), None), DropKind::TorrentFiles);
        assert_eq!(
            classify_drop(None, Some("  magnet:?xt=urn:btih:abc")),
            DropKind::Magnet
        );
        assert_eq!(classify_drop(None, Some("hello")), DropKind::Other);
    }

    /// A daemon whose `/auth/login` answers `login`, with the snapshot routes the main screen polls.
    fn daemon(runtime: &tokio::runtime::Runtime, login: u16) -> MockServer {
        const FIXTURES: &str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../rtbit-client/tests/fixtures"
        );
        let json = |name: &str| {
            let body = std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap();
            ResponseTemplate::new(200).set_body_raw(body, "application/json")
        };
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
            let login = if login == 200 {
                json("login.json")
            } else {
                ResponseTemplate::new(login)
            };
            Mock::given(method("POST"))
                .and(path("/auth/login"))
                .respond_with(login)
                .mount(&server)
                .await;
            for (route, file) in [("/torrents", "torrents.json"), ("/stats", "stats.json")] {
                Mock::given(method("GET"))
                    .and(path(route))
                    .respond_with(json(file))
                    .mount(&server)
                    .await;
            }
            server
        })
    }

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

    fn shell(cx: &mut TestAppContext) -> (Entity<Shell>, Entity<StatusBar>) {
        let status = cx.new(|_| StatusBar::new());
        let main = cx.new(|cx| MainScreen::new(status.clone(), cx));
        let shell = cx.new(|cx| Shell::new(status.clone(), main, cx));
        (shell, status)
    }

    fn sign_in(shell: &Entity<Shell>, cx: &mut TestAppContext, url: &str, pass: &str) {
        let connect = shell.read_with(cx, |s, _| s.connect.clone());
        connect.update(cx, |screen, cx| {
            screen.fill_for_test(cx, url, "admin", pass);
            screen.submit(cx);
        });
        drive(cx, |cx| connect.read_with(cx, |s, _| !s.is_pending()));
    }

    /// Sign in, then start the main screen's poll (render does this in the app).
    fn signed_in(
        cx: &mut TestAppContext,
        runtime: &tokio::runtime::Runtime,
    ) -> (Entity<Shell>, Entity<StatusBar>, MockServer) {
        let server = daemon(runtime, 200);
        let (shell, status) = shell(cx);
        sign_in(&shell, cx, &server.uri(), "secret");
        let main = shell.read_with(cx, |s, _| s.main.clone());
        main.update(cx, |main, cx| main.start_polling_for_test(cx));
        (shell, status, server)
    }

    /// Step the fake clock a second at a time until `done`.
    fn tick_until(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
        for _ in 0..1500 {
            cx.executor().advance_clock(Duration::from_secs(1));
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("condition never reached");
    }

    fn failing_routes(runtime: &tokio::runtime::Runtime, server: &MockServer, status: u16) {
        runtime.block_on(async {
            server.reset().await;
            for route in ["/torrents", "/stats"] {
                // A 401 triggers one refresh; the daemon refuses that too.
                Mock::given(method("GET"))
                    .and(path(route))
                    .respond_with(ResponseTemplate::new(status))
                    .mount(server)
                    .await;
            }
            Mock::given(method("POST"))
                .and(path("/auth/refresh"))
                .respond_with(ResponseTemplate::new(401))
                .mount(server)
                .await;
        });
    }

    #[gpui::test]
    fn lost_daemon_shows_banner_then_clears_on_recovery(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (shell, status, server) = signed_in(cx, &runtime);
        let main = shell.read_with(cx, |s, _| s.main.clone());
        tick_until(cx, |cx| main.read_with(cx, |m, _| m.torrent_count() > 0));
        assert_eq!(shell.read_with(cx, |s, _| s.lost_banner()), None);

        failing_routes(&runtime, &server, 503);
        tick_until(cx, |cx| {
            shell.read_with(cx, |s, _| s.lost_banner().is_some())
        });
        let banner = shell.read_with(cx, |s, _| s.lost_banner()).unwrap();
        assert!(
            banner.starts_with("Lost connection to daemon — retrying in "),
            "{banner}"
        );
        assert!(main.read_with(cx, |m, _| m.daemon_lost()));
        assert_eq!(shell.read_with(cx, |s, _| s.screen()), Screen::Main);

        // Actions that need the daemon are refused while it is lost.
        let before = runtime.block_on(server.received_requests()).unwrap().len();
        main.update(cx, |m, cx| m.open_add(cx));
        assert!(!main.read_with(cx, |m, _| m.add_open()));
        assert_eq!(
            runtime.block_on(server.received_requests()).unwrap().len(),
            before
        );

        // The first good poll clears the banner.
        runtime.block_on(async {
            server.reset().await;
            for (route, file) in [("/torrents", "torrents.json"), ("/stats", "stats.json")] {
                let body = std::fs::read_to_string(format!(
                    "{}/../rtbit-client/tests/fixtures/{file}",
                    env!("CARGO_MANIFEST_DIR")
                ))
                .unwrap();
                Mock::given(method("GET"))
                    .and(path(route))
                    .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
                    .mount(&server)
                    .await;
            }
        });
        tick_until(cx, |cx| {
            shell.read_with(cx, |s, _| s.lost_banner().is_none())
        });
        assert!(!main.read_with(cx, |m, _| m.daemon_lost()));
        assert_eq!(
            status.read_with(cx, |s, _| s.connection()),
            Connection::Connected
        );
    }

    #[gpui::test]
    fn unauthorised_poll_routes_back_to_connect(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (shell, status, server) = signed_in(cx, &runtime);
        let main = shell.read_with(cx, |s, _| s.main.clone());
        tick_until(cx, |cx| main.read_with(cx, |m, _| m.torrent_count() > 0));
        failing_routes(&runtime, &server, 401);
        tick_until(cx, |cx| {
            shell.read_with(cx, |s, _| s.screen() == Screen::Connect)
        });
        assert_eq!(shell.read_with(cx, |s, _| s.lost_banner()), None);
        assert_eq!(
            status.read_with(cx, |s, _| s.connection()),
            Connection::Disconnected
        );
    }

    /// Screens that load on selection need a bridge; this one points nowhere.
    fn install_idle_bridge(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        cx.update(|cx| {
            crate::bridge::Bridge::new("http://127.0.0.1:1")
                .unwrap()
                .install(cx)
        });
    }

    #[gpui::test]
    fn no_banner_while_reachable(cx: &mut TestAppContext) {
        let (shell, _status) = shell(cx);
        assert_eq!(shell.read_with(cx, |s, _| s.lost_banner()), None);
    }

    #[gpui::test]
    fn daemon_lost_event_sets_banner_and_error_badge_then_recovers(cx: &mut TestAppContext) {
        let (shell, status) = shell(cx);
        shell.update(cx, |s, cx| {
            s.on_main_event(MainEvent::DaemonLost { retry_in_secs: 8 }, cx)
        });
        assert_eq!(
            shell.read_with(cx, |s, _| s.lost_banner()).as_deref(),
            Some("Lost connection to daemon — retrying in 8s")
        );
        assert_eq!(
            status.read_with(cx, |s, _| s.connection()),
            Connection::Error
        );
        shell.update(cx, |s, cx| s.on_main_event(MainEvent::DaemonRecovered, cx));
        assert_eq!(shell.read_with(cx, |s, _| s.lost_banner()), None);
        assert_eq!(
            status.read_with(cx, |s, _| s.connection()),
            Connection::Connected
        );
    }

    #[gpui::test]
    fn unauthorised_event_returns_to_connect(cx: &mut TestAppContext) {
        install_idle_bridge(cx);
        let (shell, status) = shell(cx);
        shell.update(cx, |s, cx| {
            s.select_screen("settings", cx);
            s.on_main_event(MainEvent::Unauthorised, cx);
        });
        assert_eq!(shell.read_with(cx, |s, _| s.screen()), Screen::Connect);
        assert_eq!(
            status.read_with(cx, |s, _| s.connection()),
            Connection::Disconnected
        );
    }

    #[gpui::test]
    fn connect_events_route_screens_and_badge(cx: &mut TestAppContext) {
        let (shell, status) = shell(cx);
        let badge = |cx: &mut TestAppContext| status.read_with(cx, |s, _| s.connection());
        shell.update(cx, |s, cx| s.on_connect_event(ConnectEvent::Connecting, cx));
        assert_eq!(badge(cx), Connection::Connecting);
        shell.update(cx, |s, cx| {
            s.on_connect_event(ConnectEvent::SetupRequired, cx)
        });
        assert_eq!(shell.read_with(cx, |s, _| s.screen()), Screen::Setup);
        assert_eq!(badge(cx), Connection::Disconnected);
        shell.update(cx, |s, cx| s.on_connect_event(ConnectEvent::Connected, cx));
        assert_eq!(shell.read_with(cx, |s, _| s.screen()), Screen::Main);
        shell.update(cx, |s, cx| s.on_connect_event(ConnectEvent::SignedOut, cx));
        assert_eq!(shell.read_with(cx, |s, _| s.screen()), Screen::Connect);
    }

    #[gpui::test]
    fn select_filter_keeps_screen_and_ignores_unknown_labels(cx: &mut TestAppContext) {
        install_idle_bridge(cx);
        let (shell, _status) = shell(cx);
        shell.update(cx, |s, cx| s.select_screen("rss", cx));
        let target = StatusFilter::ALL[1];
        shell.update(cx, |s, cx| s.select_filter(target.label(), cx));
        assert_eq!(shell.read_with(cx, |s, _| s.filter), target);
        shell.update(cx, |s, cx| s.select_filter("no such filter", cx));
        assert_eq!(shell.read_with(cx, |s, _| s.filter), target);
        assert_eq!(shell.read_with(cx, |s, _| s.screen()), Screen::Rss);
    }

    #[gpui::test]
    fn nav_ids_are_unique_and_unknown_id_keeps_screen(cx: &mut TestAppContext) {
        let ids: Vec<_> = NAV.iter().map(|(id, _, _)| *id).collect();
        let mut dedup = ids.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(ids.len(), dedup.len());
        let (shell, _status) = shell(cx);
        shell.update(cx, |s, cx| s.select_screen("nope", cx));
        assert_eq!(shell.read_with(cx, |s, _| s.screen()), Screen::Connect);
    }

    #[gpui::test]
    fn starts_on_connect_disconnected(cx: &mut TestAppContext) {
        let (shell, status) = shell(cx);
        assert_eq!(shell.read_with(cx, |s, _| s.screen()), Screen::Connect);
        assert_eq!(
            status.read_with(cx, |s, _| s.connection()),
            Connection::Disconnected
        );
    }

    #[gpui::test]
    fn failed_login_stays_on_connect_with_an_error_badge(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let server = daemon(&runtime, 401);
        let (shell, status) = shell(cx);
        sign_in(&shell, cx, &server.uri(), "wrong");
        assert_eq!(shell.read_with(cx, |s, _| s.screen()), Screen::Connect);
        assert_eq!(
            status.read_with(cx, |s, _| s.connection()),
            Connection::Error
        );
        let connect = shell.read_with(cx, |s, _| s.connect.clone());
        connect.read_with(cx, |s, _| {
            assert_eq!(s.error(), Some("Sign-in was refused."))
        });
    }

    #[gpui::test]
    fn good_login_opens_main_and_nav_switches_screens(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let server = daemon(&runtime, 200);
        let (shell, status) = shell(cx);
        sign_in(&shell, cx, &server.uri(), "secret");
        assert_eq!(shell.read_with(cx, |s, _| s.screen()), Screen::Main);
        assert_eq!(
            status.read_with(cx, |s, _| s.connection()),
            Connection::Connected
        );
        shell.update(cx, |s, cx| s.select_screen("nope", cx));
        assert_eq!(shell.read_with(cx, |s, _| s.screen()), Screen::Main);
        shell.update(cx, |s, cx| s.select_screen("settings", cx));
        assert_eq!(shell.read_with(cx, |s, _| s.screen()), Screen::Settings);
        shell.update(cx, |s, cx| s.select_screen("torrents", cx));
        assert_eq!(shell.read_with(cx, |s, _| s.screen()), Screen::Main);
    }
}
