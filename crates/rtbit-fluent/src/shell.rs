//! Window shell: navigation sidebar, screen routing and status bar, inside a `Workspace`.

use fluent_app::WindowTitle;
use fluent_layout::{SelectableList, SelectableListItem};
use gpui::{
    AnyElement, Context, Entity, IntoElement, Render, Subscription, Window, div, prelude::*, px,
};
use rtbit_ui_core::{Connection, Screen, Snapshot, StatusFilter};

use crate::screens::connect::{ConnectEvent, ConnectScreen};
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
    _connect_events: Subscription,
}

impl Shell {
    #[must_use]
    pub fn new(status: Entity<StatusBar>, cx: &mut Context<Self>) -> Self {
        let connect = cx.new(ConnectScreen::new);
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
            ConnectEvent::Failed => self.set_connection(Connection::Error, cx),
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
            cx.notify();
        }
    }

    /// Set the sidebar status filter by its label. Unknown labels are ignored.
    pub fn select_filter(&mut self, label: &str, cx: &mut Context<Self>) {
        if let Some(filter) = StatusFilter::ALL.iter().find(|f| f.label() == label) {
            self.filter = *filter;
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
        if self.screen == Screen::Connect {
            return div()
                .flex_1()
                .p_4()
                .child(self.connect.clone())
                .into_any_element();
        }
        let text = match self.screen {
            Screen::Connect => "Connect to a daemon".to_owned(),
            Screen::Setup => "First-run setup".to_owned(),
            Screen::Main => format!("Torrents · {}", self.filter.label()),
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
