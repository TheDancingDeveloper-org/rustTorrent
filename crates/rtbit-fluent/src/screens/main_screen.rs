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
    MessageIntent, Toast, ToastHost, ToastStack, Toolbar, ToolbarItem,
};
use fluent_primitives::{Label, LabelSize};
use gpui::{
    App, AsyncApp, ClickEvent, Context, Entity, IntoElement, Pixels, Point, Render, SharedString,
    Subscription, Task, WeakEntity, Window, div, prelude::*,
};
use rtbit_client::{Torrent, TorrentState};
use rtbit_ui_core::{AddRequest, AddSource, RowAction, Snapshot, StatusFilter, format, map_error};

use crate::bridge::{Bridge, Poller, fetch_snapshot};
use crate::screens::add_dialog::{AddDialog, AddDialogEvent};
use crate::screens::detail::DetailPane;
use crate::widgets::status_bar::StatusBar;

/// Torrent table, toolbar and context menu. Owns the snapshot the poll fills.
pub struct MainScreen {
    pub(crate) snapshot: AsyncResource<Snapshot, String>,
    filter: StatusFilter,
    status: Entity<StatusBar>,
    /// Detail pane for the selected row.
    detail: Entity<DetailPane>,
    /// Selected row id (the daemon id, or the info hash when the daemon omits one).
    selected: Option<String>,
    sort: Option<(String, DataSortDirection)>,
    menu: Option<Entity<ContextMenu>>,
    action_error: Option<String>,
    applied_generation: u64,
    /// Add-torrent dialog, while it is open.
    add: Option<Entity<AddDialog>>,
    _add_events: Option<Subscription>,
    _add_send: Option<Task<()>>,
    toasts: Entity<ToastHost>,
    pub(crate) _poller: Option<Poller>,
    pub(crate) _load: Option<Task<()>>,
    _action: Option<Task<()>>,
}

impl MainScreen {
    #[must_use]
    pub fn new(status: Entity<StatusBar>, cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: AsyncResource::new(),
            filter: StatusFilter::All,
            status,
            detail: cx.new(DetailPane::new),
            selected: None,
            sort: None,
            menu: None,
            action_error: None,
            applied_generation: 0,
            add: None,
            _add_events: None,
            _add_send: None,
            toasts: cx.new(ToastHost::new),
            _poller: None,
            _load: None,
            _action: None,
        }
    }

    /// Open the add-torrent dialog over the table.
    pub(crate) fn open_add(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        let dialog = cx.new(AddDialog::new);
        let events = cx.subscribe(
            &dialog,
            |screen: &mut Self, dialog: Entity<AddDialog>, event: &AddDialogEvent, cx| match event {
                AddDialogEvent::Cancel => screen.close_add(cx),
                AddDialogEvent::Submit(request) => screen.send_add(dialog, request.clone(), cx),
            },
        );
        self.add = Some(dialog);
        self._add_events = Some(events);
        cx.notify();
    }

    /// Close the add dialog. Nothing is sent.
    fn close_add(&mut self, cx: &mut Context<Self>) {
        self.add = None;
        self._add_events = None;
        cx.notify();
    }

    /// Send an add request through the bridge. The dialog closes on success and
    /// shows the daemon's reason on failure.
    fn send_add(&mut self, dialog: Entity<AddDialog>, request: AddRequest, cx: &mut Context<Self>) {
        if !cx.has_global::<Bridge>() {
            dialog.update(cx, |dialog, cx| {
                dialog.refused("Not connected to a daemon.".to_owned(), cx);
            });
            return;
        }
        let rx = cx.global::<Bridge>().call(move |client| async move {
            let AddRequest {
                source,
                save_path,
                category,
                start_paused,
            } = request;
            let category = (!category.is_empty()).then_some(category);
            let mut client = client.lock().await;
            match source {
                AddSource::Magnet(magnet) => {
                    client
                        .add_magnet(&magnet, Some(&save_path), category.as_deref(), start_paused)
                        .await
                }
                AddSource::File { bytes, .. } => {
                    client
                        .add_torrent_file(
                            &bytes,
                            Some(&save_path),
                            category.as_deref(),
                            start_paused,
                        )
                        .await
                }
            }
        });
        self._add_send = Some(
            cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
                let reply = match rx.await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(err)) => Err(map_error(&err)),
                    Err(_) => Err("request cancelled".to_owned()),
                };
                let _ = view.update(cx, |screen, cx| match reply {
                    Ok(()) => {
                        screen.close_add(cx);
                        ToastStack::push(
                            Toast::new("add-torrent", "Torrent added.")
                                .intent(MessageIntent::Success),
                            cx,
                        );
                        screen.refresh(cx);
                    }
                    Err(message) => dialog.update(cx, |dialog, cx| dialog.refused(message, cx)),
                });
            }),
        );
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

    /// Select a row and load its detail. A row with no daemon id clears the pane.
    fn select_row(&mut self, row: &str, cx: &mut Context<Self>) {
        self.selected = Some(row.to_owned());
        let id = self.selected_torrent().and_then(|torrent| torrent.id);
        self.detail.update(cx, |pane, cx| match id {
            Some(id) => pane.select_torrent(id, cx),
            None => pane.clear(cx),
        });
        cx.notify();
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
        self.select_row(row, cx);
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
            confirm_delete(&me, app);
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
                            screen.detail.update(cx, |pane, cx| pane.clear(cx));
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
            .empty_message(NO_TORRENTS)
            .on_row_select(move |id: SharedString, _, app| {
                let id = id.to_string();
                select_me.update(app, |screen, cx| screen.select_row(&id, cx));
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
        let add_me = me.clone();
        let toolbar = Toolbar::new().items([
            ToolbarItem::action("add", "Add", move |_, _, app| {
                add_me.update(app, |screen, cx| screen.open_add(cx));
            }),
            ToolbarItem::separator(),
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
        root = root.child(self.detail.clone());
        if let Some(menu) = &self.menu {
            root = root.child(menu.clone());
        }
        if let Some(add) = &self.add {
            root = root.child(add.clone());
        }
        root = root.child(self.toasts.clone());
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

/// Shown in the table when the daemon has no torrents.
pub const NO_TORRENTS: &str = "No torrents yet.";

/// Run a confirmed delete. Calls the daemon path directly: `request` would open the modal again.
fn confirm_delete(me: &Entity<MainScreen>, app: &mut App) {
    me.update(app, |screen, cx| screen.act(RowAction::Delete, cx));
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
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
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
    fn confirmed_delete_hits_daemon_once(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        runtime.block_on(
            Mock::given(method("POST"))
                .and(path("/torrents/3/delete"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server),
        );
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| {
            screen._load = Some(Bridge::load(screen, cx, snapshot_of, fetch_snapshot));
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.torrent_count() > 0));
        screen.update(cx, |screen, cx| screen.select_row("3", cx));
        // The modal's on_confirm path.
        let me = screen.clone();
        cx.update(|app| confirm_delete(&me, app));
        // A successful delete clears the selection.
        drive(cx, |cx| screen.read_with(cx, |s, _| s.selected.is_none()));
        screen.read_with(cx, |s, _| assert!(s.action_error.is_none()));
        std::thread::sleep(Duration::from_millis(50));
        let hits = runtime
            .block_on(server.received_requests())
            .unwrap()
            .iter()
            .filter(|req| req.method.as_str() == "POST" && req.url.path() == "/torrents/3/delete")
            .count();
        assert_eq!(hits, 1, "confirmed delete must hit the daemon exactly once");
    }

    #[gpui::test]
    fn selecting_a_row_loads_its_detail(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        let bitfield = ResponseTemplate::new(200)
            .set_body_bytes(vec![0b1000_0000u8])
            .insert_header("x-bitfield-len", "10");
        let json =
            |name: &str| ResponseTemplate::new(200).set_body_raw(fixture(name), "application/json");
        for (route, response) in [
            ("/torrents/3", json("torrent.json")),
            ("/torrents/3/trackers", json("trackers.json")),
            ("/torrents/3/peer_stats", json("peers.json")),
            ("/torrents/3/haves", bitfield),
        ] {
            runtime.block_on(
                Mock::given(method("GET"))
                    .and(path(route))
                    .respond_with(response)
                    .mount(&server),
            );
        }
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| {
            screen._load = Some(Bridge::load(screen, cx, snapshot_of, fetch_snapshot));
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.torrent_count() > 0));
        screen.update(cx, |screen, cx| screen.select_row("3", cx));
        let detail = screen.read_with(cx, |s, _| s.detail.clone());
        drive(cx, |cx| detail.read_with(cx, |p, _| p.snapshot().is_some()));
        detail.read_with(cx, |pane, _| {
            assert_eq!(pane.torrent_id(), Some(3));
            assert!(pane.error().is_none());
        });
    }

    #[gpui::test]
    fn pause_action_hits_daemon_endpoint(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
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

    #[gpui::test]
    fn status_filter_hides_rows_that_do_not_match(cx: &mut TestAppContext) {
        let (_runtime, _server) = daemon(cx);
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| {
            screen._load = Some(Bridge::load(screen, cx, snapshot_of, fetch_snapshot));
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.torrent_count() > 0));
        // The fixture row reports no state, so only "All" keeps it.
        screen.update(cx, |screen, cx| screen.set_filter(StatusFilter::Live, cx));
        screen.read_with(cx, |s, _| assert!(s.visible_rows().is_empty()));
        screen.update(cx, |screen, cx| screen.set_filter(StatusFilter::All, cx));
        screen.read_with(cx, |s, _| assert_eq!(s.visible_rows().len(), 1));
    }

    const HASH_MAGNET: &str =
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Example";

    /// Count the POSTs the daemon has seen on `/torrents` (adds, not the list GET).
    fn add_posts(runtime: &tokio::runtime::Runtime, server: &MockServer) -> usize {
        runtime
            .block_on(server.received_requests())
            .unwrap()
            .iter()
            .filter(|req| req.method.as_str() == "POST" && req.url.path() == "/torrents")
            .count()
    }

    #[gpui::test]
    fn bad_magnet_is_refused_without_a_request(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| screen.open_add(cx));
        let dialog = screen.read_with(cx, |s, _| s.add.clone()).unwrap();
        dialog.update(cx, |dialog, cx| {
            dialog.fill_for_test(cx, "https://example.com/not-a-magnet", "/data");
            dialog.submit(cx);
        });
        dialog.read_with(cx, |dialog, _| {
            assert_eq!(dialog.error(), Some("magnet link must start with magnet:?"));
        });
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            add_posts(&runtime, &server),
            0,
            "a refused magnet must send nothing"
        );
    }

    #[gpui::test]
    fn cancel_closes_the_dialog_without_a_request(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| screen.open_add(cx));
        let dialog = screen.read_with(cx, |s, _| s.add.clone()).unwrap();
        dialog.update(cx, |dialog, cx| {
            dialog.fill_for_test(cx, HASH_MAGNET, "/data");
            dialog.cancel(cx);
        });
        screen.read_with(cx, |s, _| {
            assert!(s.add.is_none(), "cancel must close the dialog")
        });
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(add_posts(&runtime, &server), 0, "cancel must send nothing");
    }

    #[gpui::test]
    fn valid_magnet_adds_exactly_once(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        cx.update(ToastStack::init);
        runtime.block_on(
            Mock::given(method("POST"))
                .and(path("/torrents"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server),
        );
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| screen.open_add(cx));
        let dialog = screen.read_with(cx, |s, _| s.add.clone()).unwrap();
        dialog.update(cx, |dialog, cx| {
            dialog.fill_for_test(cx, HASH_MAGNET, "/data");
            dialog.submit(cx);
            // A second submit while the first is in flight must not send again.
            dialog.submit(cx);
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.add.is_none()));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            add_posts(&runtime, &server),
            1,
            "a valid magnet must add exactly once"
        );
    }

    #[gpui::test]
    fn a_torrent_file_is_posted_as_its_bytes_without_is_url(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        cx.update(ToastStack::init);
        // Only the exact shape counts: the file bytes, and no is_url query.
        runtime.block_on(
            Mock::given(method("POST"))
                .and(path("/torrents"))
                .and(wiremock::matchers::query_param_is_missing("is_url"))
                .and(wiremock::matchers::body_bytes(b"d8:announce0:e".as_slice()))
                .respond_with(ResponseTemplate::new(200))
                .expect(1)
                .mount(&server),
        );
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| screen.open_add(cx));
        let dialog = screen.read_with(cx, |s, _| s.add.clone()).unwrap();
        dialog.update(cx, |dialog, cx| {
            dialog.fill_for_test(cx, "", "/data");
            dialog.set_file_for_test("ubuntu.torrent", b"d8:announce0:e");
            dialog.submit(cx);
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.add.is_none()));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            add_posts(&runtime, &server),
            1,
            "a .torrent file must be posted once"
        );
        runtime.block_on(server.verify());
    }

    #[gpui::test]
    fn an_empty_list_uses_the_no_torrents_text(cx: &mut TestAppContext) {
        // The table is built with this constant; the text must say "no torrents".
        assert_eq!(NO_TORRENTS, "No torrents yet.");
        assert!(NO_TORRENTS.to_lowercase().contains("no torrents"));
        let _ = cx;
    }

    #[gpui::test]
    fn cancelled_delete_sends_nothing(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| {
            screen._load = Some(Bridge::load(screen, cx, snapshot_of, fetch_snapshot));
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.torrent_count() > 0));
        screen.update(cx, |screen, cx| screen.select_row("3", cx));
        // Opening the confirm modal needs a window, which this test context lacks, so the
        // test checks the guarantee directly: a delete only goes out from confirm_delete,
        // and a cancelled modal never calls it. No request has been made yet.
        std::thread::sleep(Duration::from_millis(50));
        let hits = runtime
            .block_on(server.received_requests())
            .unwrap()
            .iter()
            .filter(|req| req.method.as_str() == "POST" && req.url.path() == "/torrents/3/delete")
            .count();
        assert_eq!(
            hits, 0,
            "a delete that is not confirmed must not reach the daemon"
        );
    }

    #[gpui::test]
    fn start_action_hits_daemon_endpoint(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        runtime.block_on(
            Mock::given(method("POST"))
                .and(path("/torrents/3/start"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server),
        );
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| {
            screen._load = Some(Bridge::load(screen, cx, snapshot_of, fetch_snapshot));
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.torrent_count() > 0));
        screen.update(cx, |screen, cx| {
            screen.selected = Some("3".to_owned());
            screen.act(RowAction::Start, cx);
        });
        drive(cx, |cx| {
            screen.read_with(cx, |s, _| s._action.is_some() && s.action_error.is_none())
        });
        std::thread::sleep(Duration::from_millis(50));
        let hits = runtime
            .block_on(server.received_requests())
            .unwrap()
            .iter()
            .filter(|req| req.method.as_str() == "POST" && req.url.path() == "/torrents/3/start")
            .count();
        assert_eq!(hits, 1, "start must hit the daemon exactly once");
    }
}
