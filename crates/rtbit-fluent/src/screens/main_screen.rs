//! Main screen: the live torrent table, toolbar actions and the row context menu.
//!
//! Rows come from the bridge snapshot poll. Sorting happens here, not in
//! `DataTable`: `DataTable` sorts by display text, which orders sizes and
//! speeds wrongly. `DataTable` has no right-click hook, so the row context
//! menu opens on a double click.

use std::cmp::Ordering;

use fluent_core::AsyncResource;
use fluent_layout::{
    ConfirmModal, ContextMenu, DataCell, DataColumn, DataRow, DataSortDirection, DataTable,
    Toolbar, ToolbarItem,
};
use fluent_primitives::{Label, LabelSize};
use gpui::{
    App, AsyncApp, ClickEvent, Context, Entity, IntoElement, Pixels, Point, Render, SharedString,
    Task, WeakEntity, Window, div, prelude::*,
};
use rtbit_client::{Torrent, TorrentState};
use rtbit_ui_core::{RowAction, Snapshot, StatusFilter, format, map_error};

use crate::bridge::{Bridge, Poller, fetch_snapshot};
use crate::widgets::status_bar::StatusBar;

/// Torrent table, toolbar and context menu. Owns the snapshot the poll fills.
pub struct MainScreen {
    pub(crate) snapshot: AsyncResource<Snapshot, String>,
    filter: StatusFilter,
    status: Entity<StatusBar>,
    /// Selected row id (the daemon id, or the info hash when the daemon omits one).
    selected: Option<String>,
    sort: Option<(String, DataSortDirection)>,
    menu: Option<Entity<ContextMenu>>,
    action_error: Option<String>,
    applied_generation: u64,
    pub(crate) _poller: Option<Poller>,
    pub(crate) _load: Option<Task<()>>,
    _action: Option<Task<()>>,
}

impl MainScreen {
    #[must_use]
    pub fn new(status: Entity<StatusBar>) -> Self {
        Self {
            snapshot: AsyncResource::new(),
            filter: StatusFilter::All,
            status,
            selected: None,
            sort: None,
            menu: None,
            action_error: None,
            applied_generation: 0,
            _poller: None,
            _load: None,
            _action: None,
        }
    }

    /// Set the sidebar status filter the table shows.
    pub fn set_filter(&mut self, filter: StatusFilter, cx: &mut Context<Self>) {
        self.filter = filter;
        cx.notify();
    }

    /// Number of torrents in the last good snapshot, before filters.
    #[must_use]
    pub fn torrent_count(&self) -> usize {
        self.snapshot
            .value()
            .map_or(0, |snapshot| snapshot.torrents.torrents.len())
    }

    fn selected_torrent(&self) -> Option<&Torrent> {
        let selected = self.selected.as_deref()?;
        self.snapshot
            .value()?
            .torrents
            .torrents
            .iter()
            .find(|torrent| row_id(torrent) == selected)
    }

    /// Ask for a fresh snapshot now, in addition to the poll.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if !cx.has_global::<Bridge>() {
            return;
        }
        self._load = Some(Bridge::load(self, cx, snapshot_of, fetch_snapshot));
    }

    /// Start the poll once a bridge is installed. Idempotent.
    fn ensure_polling(&mut self, cx: &mut Context<Self>) {
        if self._poller.is_none() && cx.has_global::<Bridge>() {
            self._poller = Some(Bridge::poll_snapshot(cx, snapshot_of));
        }
    }

    /// Push each newly resolved snapshot to the status bar once.
    fn apply_to_status(&mut self, cx: &mut Context<Self>) {
        let generation = self.snapshot.generation();
        if generation == self.applied_generation {
            return;
        }
        self.applied_generation = generation;
        if let Some(snapshot) = self.snapshot.ready_value().cloned() {
            let status = self.status.clone();
            status.update(cx, |bar, cx| bar.apply_snapshot(&snapshot, cx));
        }
    }

    fn open_menu(&mut self, row: &str, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.selected = Some(row.to_owned());
        let screen = cx.entity();
        let (resume, pause, delete) = (screen.clone(), screen.clone(), screen);
        let menu = cx.new(|_| {
            ContextMenu::build(position)
                .action("Resume", move |_, _, app| {
                    dispatch(&resume, app, RowAction::Start);
                })
                .action("Pause", move |_, _, app| {
                    dispatch(&pause, app, RowAction::Pause);
                })
                .separator()
                .action("Delete…", move |_, _, app| {
                    dispatch(&delete, app, RowAction::Delete);
                })
        });
        self.menu = Some(menu);
        cx.notify();
    }

    /// Run a toolbar or menu command. Delete asks first.
    fn request(&mut self, action: RowAction, cx: &mut Context<Self>) {
        self.menu = None;
        let Some(name) = self.selected_torrent().map(torrent_name) else {
            self.action_error = Some("Select a torrent first.".to_owned());
            cx.notify();
            return;
        };
        if action != RowAction::Delete {
            self.act(action, cx);
            return;
        }
        let me = cx.entity();
        ConfirmModal::new(
            "Delete torrent?",
            format!("“{name}” will be removed from the daemon."),
        )
        .confirm_label("Delete")
        .on_confirm(move |app| {
            dispatch(&me, app, RowAction::Delete);
        })
        .show(cx);
        cx.notify();
    }

    /// Send one row command to the daemon, then refresh the table.
    fn act(&mut self, action: RowAction, cx: &mut Context<Self>) {
        self.menu = None;
        let Some(id) = self.selected_torrent().and_then(|torrent| torrent.id) else {
            self.action_error = Some("Select a torrent first.".to_owned());
            cx.notify();
            return;
        };
        if !cx.has_global::<Bridge>() {
            self.action_error = Some("Not connected to a daemon.".to_owned());
            cx.notify();
            return;
        }
        let rx = cx.global::<Bridge>().call(move |client| async move {
            let mut client = client.lock().await;
            match action {
                RowAction::Start => client.start_torrent(id).await,
                RowAction::Pause => client.pause_torrent(id).await,
                RowAction::Delete => client.delete_torrent(id).await,
            }
        });
        self._action = Some(
            cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
                let reply = match rx.await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(err)) => Err(map_error(&err)),
                    Err(_) => Err("request cancelled".to_owned()),
                };
                let _ = view.update(cx, |screen, cx| match reply {
                    Ok(()) => {
                        screen.action_error = None;
                        if action == RowAction::Delete {
                            screen.selected = None;
                        }
                        screen.refresh(cx);
                    }
                    Err(message) => {
                        screen.action_error = Some(message);
                        cx.notify();
                    }
                });
            }),
        );
    }

    fn sort_by(&mut self, column: &str, direction: DataSortDirection, cx: &mut Context<Self>) {
        self.sort = Some((column.to_owned(), direction));
        cx.notify();
    }

    /// Rows for the table: filtered by the sidebar status, then sorted by the column.
    fn visible_rows(&self) -> Vec<&Torrent> {
        let Some(snapshot) = self.snapshot.value() else {
            return Vec::new();
        };
        let mut rows: Vec<&Torrent> = snapshot
            .torrents
            .torrents
            .iter()
            .filter(|torrent| matches_filter(self.filter, torrent))
            .collect();
        if let Some((column, direction)) = &self.sort {
            rows.sort_by(|a, b| {
                let order = column_order(column, a, b);
                match direction {
                    DataSortDirection::Ascending => order,
                    DataSortDirection::Descending => order.reverse(),
                }
            });
        }
        rows
    }

    fn table(&self, me: &Entity<Self>) -> DataTable {
        let columns = [
            DataColumn::new("name", "Name").width(320.0).sortable(true),
            DataColumn::new("size", "Size").width(90.0).sortable(true),
            DataColumn::new("progress", "Progress")
                .width(90.0)
                .sortable(true),
            DataColumn::new("speed", "Down").width(110.0).sortable(true),
            DataColumn::new("eta", "ETA").width(90.0).sortable(true),
            DataColumn::new("state", "State")
                .width(110.0)
                .sortable(true),
        ];
        let rows = self.visible_rows().into_iter().map(|torrent| {
            DataRow::new(
                row_id(torrent),
                [
                    DataCell::new(torrent_name(torrent)),
                    DataCell::new(size_text(torrent)),
                    DataCell::new(progress_text(torrent)),
                    DataCell::new(speed_text(torrent)),
                    DataCell::new(eta_text(torrent)),
                    DataCell::new(state_of(torrent)),
                ],
            )
        });

        let select_me = me.clone();
        let click_me = me.clone();
        let sort_me = me.clone();
        let mut table = DataTable::new()
            .columns(columns)
            .rows(rows)
            .empty_message("No torrents yet.")
            .on_row_select(move |id: SharedString, _, app| {
                let id = id.to_string();
                select_me.update(app, |screen, cx| {
                    screen.selected = Some(id);
                    cx.notify();
                });
            })
            .on_row_click(move |id: SharedString, event: &ClickEvent, _, app| {
                if event.click_count() < 2 {
                    return;
                }
                let id = id.to_string();
                let position = event.position();
                click_me.update(app, |screen, cx| screen.open_menu(&id, position, cx));
            })
            .on_sort(move |column: SharedString, direction, _, _, app| {
                let column = column.to_string();
                sort_me.update(app, |screen, cx| screen.sort_by(&column, direction, cx));
            });
        if let Some(selected) = &self.selected {
            table = table.selected(selected.clone());
        }
        table
    }
}

impl Render for MainScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_polling(cx);
        self.apply_to_status(cx);
        if self.menu.as_ref().is_some_and(|menu| !menu.read(cx).open) {
            self.menu = None;
        }

        let me = cx.entity();
        let toolbar = Toolbar::new().items([
            ToolbarItem::action("resume", "Resume", toolbar_click(&me, RowAction::Start)),
            ToolbarItem::action("pause", "Pause", toolbar_click(&me, RowAction::Pause)),
            ToolbarItem::separator(),
            ToolbarItem::action("delete", "Delete", toolbar_click(&me, RowAction::Delete)),
        ]);

        let message = self
            .action_error
            .clone()
            .or_else(|| self.snapshot.error().cloned());
        let mut root = div()
            .flex()
            .flex_col()
            .flex_1()
            .p_4()
            .gap_2()
            .child(toolbar);
        if let Some(message) = message {
            root = root.child(Label::new(message).size(LabelSize::Caption));
        }
        root = root.child(self.table(&me));
        if let Some(menu) = &self.menu {
            root = root.child(menu.clone());
        }
        root
    }
}

fn toolbar_click(
    me: &Entity<MainScreen>,
    action: RowAction,
) -> impl Fn(&ClickEvent, &mut gpui::Window, &mut App) + 'static {
    let me = me.clone();
    move |_, _, app| dispatch(&me, app, action)
}

/// Hand a row command to the screen entity from a toolbar, menu or dialog.
fn dispatch(me: &Entity<MainScreen>, app: &mut App, action: RowAction) {
    me.update(app, |screen, cx| screen.request(action, cx));
}

fn snapshot_of(screen: &mut MainScreen) -> &mut AsyncResource<Snapshot, String> {
    &mut screen.snapshot
}

/// Whether a torrent passes the sidebar status filter.
fn matches_filter(filter: StatusFilter, torrent: &Torrent) -> bool {
    let state = torrent.stats.as_ref().and_then(|stats| stats.state);
    match filter {
        StatusFilter::All => true,
        StatusFilter::Live => state == Some(TorrentState::Live),
        StatusFilter::Paused => state == Some(TorrentState::Paused),
        StatusFilter::Error => state == Some(TorrentState::Error),
        StatusFilter::Initializing => state == Some(TorrentState::Initializing),
    }
}

fn row_id(torrent: &Torrent) -> String {
    torrent
        .id
        .map_or_else(|| torrent.info_hash.clone(), |id| id.to_string())
}

fn torrent_name(torrent: &Torrent) -> String {
    torrent
        .name
        .clone()
        .unwrap_or_else(|| torrent.info_hash.clone())
}

fn total_bytes(torrent: &Torrent) -> u64 {
    torrent.stats.as_ref().map_or(0, |stats| stats.total_bytes)
}

fn done_fraction(torrent: &Torrent) -> f64 {
    match torrent.stats.as_ref() {
        Some(stats) if stats.total_bytes > 0 => {
            stats.progress_bytes as f64 / stats.total_bytes as f64
        }
        _ => 0.0,
    }
}

/// Live download speed in bytes per second (the daemon reports MiB/s).
fn download_bytes_per_sec(torrent: &Torrent) -> f64 {
    torrent
        .stats
        .as_ref()
        .and_then(|stats| stats.live.as_ref())
        .map_or(0.0, |live| live.download_speed.mbps * 1024.0 * 1024.0)
}

fn size_text(torrent: &Torrent) -> String {
    format::bytes(total_bytes(torrent))
}

fn progress_text(torrent: &Torrent) -> String {
    format!("{:.1}%", done_fraction(torrent) * 100.0)
}

fn speed_text(torrent: &Torrent) -> String {
    let speed = torrent
        .stats
        .as_ref()
        .and_then(|stats| stats.live.as_ref())
        .map(|live| live.download_speed.clone())
        .unwrap_or_default();
    format::speed(&speed)
}

fn state_of(torrent: &Torrent) -> String {
    format::state_label(torrent.stats.as_ref().and_then(|stats| stats.state))
}

/// Seconds left at the current download speed, or `None` when nothing is moving.
fn eta_seconds(torrent: &Torrent) -> Option<f64> {
    let stats = torrent.stats.as_ref()?;
    let remaining = stats.total_bytes.saturating_sub(stats.progress_bytes) as f64;
    let rate = download_bytes_per_sec(torrent);
    (remaining > 0.0 && rate > 0.0).then(|| remaining / rate)
}

fn eta_text(torrent: &Torrent) -> String {
    match eta_seconds(torrent) {
        None => "—".to_owned(),
        Some(secs) => {
            let total = secs.round() as u64;
            let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
            if h > 0 {
                format!("{h}h {m:02}m")
            } else if m > 0 {
                format!("{m}m {s:02}s")
            } else {
                format!("{s}s")
            }
        }
    }
}

/// Ordering for one sortable column. Unknown columns keep daemon order.
fn column_order(column: &str, a: &Torrent, b: &Torrent) -> Ordering {
    match column {
        "name" => torrent_name(a).cmp(&torrent_name(b)),
        "size" => total_bytes(a).cmp(&total_bytes(b)),
        "progress" => done_fraction(a).total_cmp(&done_fraction(b)),
        "speed" => download_bytes_per_sec(a).total_cmp(&download_bytes_per_sec(b)),
        "eta" => eta_seconds(a)
            .unwrap_or(f64::INFINITY)
            .total_cmp(&eta_seconds(b).unwrap_or(f64::INFINITY)),
        "state" => state_of(a).cmp(&state_of(b)),
        _ => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../rtbit-client/tests/fixtures"
    );

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap()
    }

    /// Serve the login, torrent list and stats fixtures, plus a pause endpoint.
    /// Returns the bridge (already logged in) and the runtime and server to inspect.
    fn daemon(cx: &mut TestAppContext) -> (tokio::runtime::Runtime, MockServer) {
        // Replies wake gpui tasks from tokio threads, which is real (not simulated) concurrency.
        cx.executor().allow_parking();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let server = runtime.block_on(async {
            let server = MockServer::start().await;
            for (route, file) in [
                ("/auth/login", "login.json"),
                ("/torrents", "torrents.json"),
                ("/stats", "stats.json"),
            ] {
                let method_name = if route == "/auth/login" {
                    "POST"
                } else {
                    "GET"
                };
                Mock::given(method(method_name))
                    .and(path(route))
                    .respond_with(
                        ResponseTemplate::new(200).set_body_raw(fixture(file), "application/json"),
                    )
                    .mount(&server)
                    .await;
            }
            Mock::given(method("POST"))
                .and(path("/torrents/3/pause"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
            server
        });
        let bridge = Bridge::new(&server.uri()).unwrap();
        // Log in on the bridge's own runtime; the test thread only waits for the reply.
        let login =
            bridge.call(|client| async move { client.lock().await.login("admin", "secret").await });
        let reply = bridge.runtime().block_on(login).unwrap();
        reply.unwrap();
        cx.update(|cx| bridge.install(cx));
        (runtime, server)
    }

    /// Step gpui until `done` holds. Replies arrive from tokio threads.
    fn drive(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
        for _ in 0..500 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("daemon reply never arrived");
    }

    /// A row with the given size, built from public fields (no JSON parser needed).
    fn row(name: &str, total_bytes: u64) -> Torrent {
        Torrent {
            id: Some(1),
            info_hash: name.to_owned(),
            name: Some(name.to_owned()),
            output_folder: String::new(),
            total_pieces: 0,
            category: None,
            tags: None,
            trackers: None,
            files: None,
            stats: Some(rtbit_client::TorrentStats {
                total_bytes,
                ..Default::default()
            }),
        }
    }

    #[test]
    fn size_sorts_numerically_not_by_text() {
        let small = row("small", 9);
        let big = row("big", 10);
        // Text order would put "10" before "9"; the column must sort by value.
        assert_eq!(column_order("size", &small, &big), Ordering::Less);
    }

    #[test]
    fn eta_is_dash_without_speed() {
        assert_eq!(eta_text(&row("idle", 100)), "—");
    }

    #[gpui::test]
    fn poll_fills_rows_and_feeds_status_bar(cx: &mut TestAppContext) {
        let (_runtime, _server) = daemon(cx);
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|_| MainScreen::new(status.clone()));
        screen.update(cx, |screen, cx| {
            screen._poller = Some(Bridge::poll_snapshot(cx, snapshot_of));
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.torrent_count() > 0));
        screen.read_with(cx, |screen, _| {
            let rows = screen.visible_rows();
            assert_eq!(rows[0].name.as_deref(), Some("example"));
        });
    }

    #[gpui::test]
    fn pause_action_hits_daemon_endpoint(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|_| MainScreen::new(status.clone()));
        screen.update(cx, |screen, cx| {
            screen._load = Some(Bridge::load(screen, cx, snapshot_of, fetch_snapshot));
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.torrent_count() > 0));
        screen.update(cx, |screen, cx| {
            screen.selected = Some("3".to_owned());
            screen.act(RowAction::Pause, cx);
        });
        drive(cx, |cx| {
            screen.read_with(cx, |s, _| s._action.is_some() && s.action_error.is_none())
        });
        // Give the daemon call a moment to land, then count the hits.
        std::thread::sleep(Duration::from_millis(50));
        let hits = runtime
            .block_on(server.received_requests())
            .unwrap()
            .iter()
            .filter(|req| req.method.as_str() == "POST" && req.url.path() == "/torrents/3/pause")
            .count();
        assert_eq!(hits, 1, "pause must hit the daemon exactly once");
    }
}
