//! Window shell: navigation sidebar, screen routing and status bar, inside a `Workspace`.

use fluent_app::WindowTitle;
use fluent_layout::{SelectableList, SelectableListItem};
use gpui::{
    AnyElement, Context, Entity, IntoElement, Render, Subscription, Window, div, prelude::*, px,
};
use rtbit_ui_core::{Connection, Screen, Snapshot, StatusFilter};

use crate::screens::MainScreen;
use crate::screens::connect::{ConnectEvent, ConnectScreen};
use crate::screens::rss::RssScreen;
use crate::screens::settings::SettingsScreen;
use crate::widgets::status_bar::StatusBar;

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
    _connect_events: Subscription,
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
        Self {
            screen: Screen::Connect,
            filter: StatusFilter::All,
            status,
            connect,
            rss: cx.new(RssScreen::new),
            settings,
            main,
            _connect_events: connect_events,
        }
    }

    /// Follow a sign-in attempt: badge and title always; the main screen on success.
    fn on_connect_event(&mut self, event: ConnectEvent, cx: &mut Context<Self>) {
        match event {
            ConnectEvent::Connecting => self.set_connection(Connection::Connecting, cx),
            ConnectEvent::Connected => {
                self.screen = Screen::Main;
                self.set_connection(Connection::Connected, cx);
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
            self.screen = *screen;
            if self.screen == Screen::Rss {
                self.rss.update(cx, |rss, cx| rss.load(cx));
            }
            if self.screen == Screen::Settings {
                self.settings.update(cx, |settings, cx| settings.load(cx));
            }
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
            .child(self.body())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gpui::TestAppContext;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

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
