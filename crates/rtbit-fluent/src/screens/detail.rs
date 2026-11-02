//! Torrent detail pane: a bottom pane with a `TabStrip` (General, Files,
//! Peers, Trackers) for the selected torrent.
//!
//! Every read and write runs on the bridge's tokio runtime. The pane keeps
//! the last good [`DetailSnapshot`] in an [`AsyncResource`], so a reply from a
//! superseded selection cannot overwrite the current one. The Files tab is a
//! `Tree` of folders and files. Double-clicking a node, or the checkbox and
//! priority buttons under the tree, posts the new selection and reloads.
//!
//! The daemon has two file priorities only: download ("Normal") and skip
//! ("Do not download"). A folder that holds both reads as "Mixed".

use std::collections::{BTreeMap, HashSet};

use fluent_core::AsyncResource;
use fluent_layout::{
    DataColumn, DataRow, DataTable, MessageBar, MessageIntent, TabItem, TabStrip, Tree, TreeItem,
};
use fluent_primitives::{Button, Checkbox, CheckboxState, Label};
use gpui::{
    App, AsyncApp, ClickEvent, Context, Entity, IntoElement, Render, SharedString, Task,
    WeakEntity, Window, div, prelude::*, px,
};
use rtbit_client::{Error, TorrentFile};
use rtbit_ui_core::{DetailSnapshot, format, map_error};

use crate::bridge::{Bridge, Poller, SharedClient};

/// Tab ids, in strip order.
const TABS: [(&str, &str); 5] = [
    ("general", "General"),
    ("files", "Files"),
    ("peers", "Peers"),
    ("trackers", "Trackers"),
    ("pieces", "Pieces"),
];

const NO_TORRENT_SELECTED: &str = "No torrent selected.";
const METADATA_RESOLVING: &str = "Torrent metadata is still resolving.";
const NO_CONNECTED_PEERS: &str = "No connected peers.";
const NO_TRACKERS: &str = "No trackers (DHT-only).";

/// The named empty state associated with each detail tab.
fn empty_state_message(tab: &str) -> &'static str {
    match tab {
        "files" | "pieces" => METADATA_RESOLVING,
        "peers" => NO_CONNECTED_PEERS,
        "trackers" => NO_TRACKERS,
        _ => NO_TORRENT_SELECTED,
    }
}

/// The detail pane for one selected torrent.
pub struct DetailPane {
    torrent_id: Option<u64>,
    detail: AsyncResource<DetailSnapshot, String>,
    tabs: Entity<TabStrip>,
    error: Option<String>,
    /// The Files tree node the checkbox and priority buttons act on.
    selected_node: Option<SharedString>,
    /// Folder nodes the user collapsed; folders start expanded.
    collapsed: HashSet<SharedString>,
    _load: Option<Task<()>>,
    _action: Option<Task<()>>,
    _poller: Option<Poller>,
}

fn detail_of(pane: &mut DetailPane) -> &mut AsyncResource<DetailSnapshot, String> {
    &mut pane.detail
}

/// Fetch everything the pane shows for one torrent, under one client lock.
async fn fetch_detail(client: SharedClient, id: u64) -> Result<DetailSnapshot, Error> {
    let mut client = client.lock().await;
    let torrent = client.torrent(id).await?;
    let trackers = client.trackers(id).await?;
    let peers = client.peers(id).await?;
    let pieces = client.haves(id).await?;
    Ok(DetailSnapshot {
        torrent,
        trackers,
        peers,
        pieces,
    })
}

impl DetailPane {
    #[must_use]
    pub fn new(cx: &mut Context<Self>) -> Self {
        let tabs = cx.new(|cx| {
            let mut strip = TabStrip::new(cx);
            for (id, label) in TABS {
                strip.add_tab(TabItem::new(id).label(label));
            }
            strip
        });
        // A tab click notifies the strip; re-render the pane so the body follows.
        cx.observe(&tabs, |_, _, cx| cx.notify()).detach();
        Self {
            torrent_id: None,
            detail: AsyncResource::default(),
            tabs,
            error: None,
            selected_node: None,
            collapsed: HashSet::new(),
            _load: None,
            _action: None,
            _poller: None,
        }
    }

    /// The torrent the pane shows, if any.
    #[must_use]
    pub fn torrent_id(&self) -> Option<u64> {
        self.torrent_id
    }

    /// The last loaded detail, if one has arrived.
    #[must_use]
    pub fn snapshot(&self) -> Option<&DetailSnapshot> {
        self.detail.ready_value()
    }

    /// The error bar text, if the last load or file post failed.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The active tab id.
    #[must_use]
    pub fn active_tab(&self, cx: &App) -> &'static str {
        TABS[self.tabs.read(cx).active.min(TABS.len() - 1)].0
    }

    /// Show torrent `id`: load its detail on the bridge. The previous detail
    /// stays until the new one lands, and superseded replies are dropped.
    pub fn select_torrent(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.torrent_id != Some(id) {
            self.selected_node = None;
            self.collapsed.clear();
        }
        self.torrent_id = Some(id);
        self.error = None;
        self._load = Some(Bridge::load(self, cx, detail_of, move |client| {
            fetch_detail(client, id)
        }));
    }

    /// Clear the selection and drop any in-flight load or post.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.torrent_id = None;
        self.detail = AsyncResource::default();
        self.error = None;
        self.selected_node = None;
        self.collapsed.clear();
        self._load = None;
        self._action = None;
        cx.notify();
    }

    /// Flip file `index` in or out of the download selection and post the
    /// whole selection to the daemon. Ignored without a loaded torrent.
    pub fn toggle_file(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(id) = self.torrent_id else {
            return;
        };
        let Some(files) = self
            .detail
            .ready_value()
            .and_then(|snapshot| snapshot.torrent.files.clone())
        else {
            return;
        };
        let Some(file) = files.get(index) else {
            return;
        };
        let included = !file.included;
        let rx = cx.global::<Bridge>().call(move |client| async move {
            let mut client = client.lock().await;
            client.set_file_included(id, index, included).await
        });
        self.await_action(id, rx, cx);
    }

    /// The selected Files tree node, if any.
    #[must_use]
    pub fn selected_node(&self) -> Option<&SharedString> {
        self.selected_node.as_ref()
    }

    /// Select a Files tree node for the checkbox and priority buttons.
    pub fn select_node(&mut self, node: SharedString, cx: &mut Context<Self>) {
        self.selected_node = Some(node);
        cx.notify();
    }

    /// Expand or collapse a folder node.
    pub fn set_expanded(&mut self, node: SharedString, expanded: bool, cx: &mut Context<Self>) {
        if expanded {
            self.collapsed.remove(&node);
        } else {
            self.collapsed.insert(node);
        }
        cx.notify();
    }

    /// Flip a tree node: a file or every file under a folder. A fully
    /// included node is skipped; anything else is included.
    pub fn toggle_node(&mut self, node: &str, cx: &mut Context<Self>) {
        let Some(files) = self.files() else {
            return;
        };
        let indices = indices_under(&files, node);
        let include = inclusion(&files, &indices) != CheckboxState::Checked;
        self.set_included(&indices, include, cx);
    }

    /// Include (priority "Normal") or skip ("Do not download") every listed
    /// file. One file posts to its own route; several post the whole new
    /// selection with `update_only_files`.
    pub fn set_included(&mut self, indices: &[usize], included: bool, cx: &mut Context<Self>) {
        let Some(id) = self.torrent_id else {
            return;
        };
        let Some(files) = self.files() else {
            return;
        };
        let indices: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|&i| files.get(i).is_some_and(|f| f.included != included))
            .collect();
        match indices.as_slice() {
            [] => {}
            [index] => {
                let index = *index;
                let rx = cx.global::<Bridge>().call(move |client| async move {
                    let mut client = client.lock().await;
                    client.set_file_included(id, index, included).await
                });
                self.await_action(id, rx, cx);
            }
            _ => {
                let selection = new_selection(&files, &indices, included);
                let rx = cx.global::<Bridge>().call(move |client| async move {
                    let mut client = client.lock().await;
                    client.update_only_files(id, &selection).await
                });
                self.await_action(id, rx, cx);
            }
        }
    }

    fn files(&self) -> Option<Vec<TorrentFile>> {
        self.detail
            .ready_value()
            .and_then(|snapshot| snapshot.torrent.files.clone())
    }

    fn await_action(
        &mut self,
        id: u64,
        rx: tokio::sync::oneshot::Receiver<Result<(), Error>>,
        cx: &mut Context<Self>,
    ) {
        self._action = Some(
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                let reply = match rx.await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(err)) => Err(map_error(&err)),
                    Err(_) => Err("request cancelled".to_owned()),
                };
                let _ = this.update(cx, |this, cx| this.finish_action(id, reply, cx));
            }),
        );
    }

    fn finish_action(&mut self, id: u64, reply: Result<(), String>, cx: &mut Context<Self>) {
        match reply {
            // Reload so the checkboxes show what the daemon now holds.
            Ok(()) if self.torrent_id == Some(id) => self.select_torrent(id, cx),
            Ok(()) => {}
            Err(message) => self.error = Some(message),
        }
        cx.notify();
    }

    fn body(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(snapshot) = self.detail.ready_value() else {
            let text = if self.torrent_id.is_some() {
                "Loading…"
            } else {
                NO_TORRENT_SELECTED
            };
            return Label::new(text).into_any_element();
        };
        match self.active_tab(cx) {
            "files" if snapshot.torrent.files.is_none() => {
                Label::new(empty_state_message("files")).into_any_element()
            }
            "files" => files(snapshot, self, cx).into_any_element(),
            "peers" => peers_table(snapshot).into_any_element(),
            "trackers" => trackers_table(snapshot).into_any_element(),
            "pieces" if snapshot.pieces.have.is_empty() => {
                Label::new(empty_state_message("pieces")).into_any_element()
            }
            "pieces" => pieces_summary(snapshot).into_any_element(),
            _ => general(snapshot).into_any_element(),
        }
    }
}

/// Tree node id of a folder (its path joined with `/`).
fn dir_id(path: &str) -> SharedString {
    format!("dir:{path}").into()
}

/// Tree node id of file `index`.
fn file_id(index: usize) -> SharedString {
    format!("file:{index}").into()
}

/// A file's path parts. Older daemons send no `components`; split the name.
fn parts(file: &TorrentFile) -> Vec<&str> {
    if file.components.is_empty() {
        file.name.split('/').filter(|p| !p.is_empty()).collect()
    } else {
        file.components.iter().map(String::as_str).collect()
    }
}

/// Indices of the files a node covers: one for a file, every file below for a folder.
fn indices_under(files: &[TorrentFile], node: &str) -> Vec<usize> {
    if let Some(index) = node.strip_prefix("file:") {
        return index
            .parse()
            .ok()
            .filter(|i| *i < files.len())
            .into_iter()
            .collect();
    }
    let Some(dir) = node.strip_prefix("dir:") else {
        return Vec::new();
    };
    let prefix: Vec<&str> = dir.split('/').collect();
    files
        .iter()
        .enumerate()
        .filter(|(_, file)| {
            let parts = parts(file);
            parts.len() > prefix.len() && parts[..prefix.len()] == prefix[..]
        })
        .map(|(i, _)| i)
        .collect()
}

/// Checkbox state of a set of files: all in, none in, or a mix.
fn inclusion(files: &[TorrentFile], indices: &[usize]) -> CheckboxState {
    let included = indices.iter().filter(|&&i| files[i].included).count();
    match included {
        0 => CheckboxState::Unchecked,
        n if n == indices.len() => CheckboxState::Checked,
        _ => CheckboxState::Indeterminate,
    }
}

/// The priority a checkbox state stands for.
fn priority_label(state: CheckboxState) -> &'static str {
    match state {
        CheckboxState::Checked => "Normal",
        CheckboxState::Unchecked => "Do not download",
        CheckboxState::Indeterminate => "Mixed",
    }
}

/// The whole selection after setting `changed` to `included`.
fn new_selection(files: &[TorrentFile], changed: &[usize], included: bool) -> Vec<usize> {
    files
        .iter()
        .enumerate()
        .filter(|(i, file)| {
            if changed.contains(i) {
                included
            } else {
                file.included
            }
        })
        .map(|(i, _)| i)
        .collect()
}

/// Folder and file nodes, folders first and sorted by name, files in torrent order.
fn file_tree(files: &[TorrentFile], collapsed: &HashSet<SharedString>) -> Vec<TreeItem> {
    let all: Vec<usize> = (0..files.len()).collect();
    level(files, &all, &[], collapsed)
}

fn level(
    files: &[TorrentFile],
    indices: &[usize],
    prefix: &[&str],
    collapsed: &HashSet<SharedString>,
) -> Vec<TreeItem> {
    let mut dirs: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut leaves = Vec::new();
    for &i in indices {
        let parts = parts(&files[i]);
        match parts.get(prefix.len()) {
            Some(name) if parts.len() > prefix.len() + 1 => dirs.entry(name).or_default().push(i),
            _ => leaves.push(i),
        }
    }
    let mut items = Vec::new();
    for (name, under) in dirs {
        let mut path = prefix.to_vec();
        path.push(name);
        let id = dir_id(&path.join("/"));
        let size: u64 = under.iter().map(|&i| files[i].length).sum();
        let badge = format!(
            "{}  {}",
            format::bytes(size),
            priority_label(inclusion(files, &under))
        );
        let mut item = TreeItem::new(id.clone(), name.to_owned())
            .badge(badge)
            .expanded(!collapsed.contains(&id));
        item.children = level(files, &under, &path, collapsed);
        items.push(item);
    }
    for i in leaves {
        let file = &files[i];
        let name = parts(file)
            .last()
            .map_or(file.name.as_str(), |n| n)
            .to_owned();
        let badge = format!(
            "{}  {}",
            format::bytes(file.length),
            priority_label(inclusion(files, &[i]))
        );
        items.push(TreeItem::new(file_id(i), name).badge(badge));
    }
    items
}

/// The file tree, then the selected node's checkbox and priority buttons.
/// Double-clicking a node flips it as the checkbox would.
fn files(
    snapshot: &DetailSnapshot,
    pane: &DetailPane,
    cx: &mut Context<DetailPane>,
) -> impl IntoElement {
    let me = cx.entity();
    let list = snapshot.torrent.files.as_deref().unwrap_or_default();
    let mut tree = Tree::new()
        .items(file_tree(list, &pane.collapsed))
        .on_select({
            let me = me.clone();
            move |node, _, _, cx| me.update(cx, |pane, cx| pane.select_node(node, cx))
        })
        .on_toggle({
            let me = me.clone();
            move |node, expanded, _, _, cx| {
                me.update(cx, |pane, cx| pane.set_expanded(node, expanded, cx));
            }
        })
        .on_activate({
            let me = me.clone();
            move |node, _, _, cx| me.update(cx, |pane, cx| pane.toggle_node(&node, cx))
        });
    if let Some(node) = &pane.selected_node {
        tree = tree.selected(node.clone());
    }
    let mut column = div().flex().flex_col().gap_2().child(tree);
    let selected = pane
        .selected_node
        .as_ref()
        .map(|node| (node.clone(), indices_under(list, node)))
        .filter(|(_, indices)| !indices.is_empty());
    if let Some((node, indices)) = selected {
        let state = inclusion(list, &indices);
        let priority_button = |id: &'static str, label: &'static str, include: bool| {
            let me = me.clone();
            let indices = indices.clone();
            Button::new(id).label(label).on_click(
                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    me.update(cx, |pane, cx| pane.set_included(&indices, include, cx));
                },
            )
        };
        let toggle = {
            let me = me.clone();
            Checkbox::new("torrent-file-node")
                .state(state)
                .on_click(move |_, _, _, cx| me.update(cx, |pane, cx| pane.toggle_node(&node, cx)))
        };
        column = column.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(toggle)
                .child(Label::new(format!("Priority: {}", priority_label(state))))
                .child(priority_button("torrent-file-normal", "Normal", true))
                .child(priority_button(
                    "torrent-file-skip",
                    "Do not download",
                    false,
                )),
        );
    }
    column
}

/// The piece bar's numbers: how many pieces are held out of the total.
fn pieces_summary(snapshot: &DetailSnapshot) -> impl IntoElement {
    let have = snapshot.pieces.have.iter().filter(|h| **h).count();
    let total = snapshot.pieces.have.len();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(Label::new(format!("Pieces held: {have} of {total}")))
}

fn general(snapshot: &DetailSnapshot) -> impl IntoElement {
    let torrent = &snapshot.torrent;
    let state = format::state_label(torrent.stats.as_ref().and_then(|stats| stats.state));
    let have = snapshot.pieces.have.iter().filter(|h| **h).count() as u64;
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(Label::new(torrent.name.clone().unwrap_or_default()))
        .child(Label::new(format!("Info hash: {}", torrent.info_hash)))
        .child(Label::new(format!("Folder: {}", torrent.output_folder)))
        .child(Label::new(format!("State: {state}")))
        .child(Label::new(format!(
            "Pieces: {}",
            format::progress(have, u64::from(torrent.total_pieces))
        )))
}

fn peers_table(snapshot: &DetailSnapshot) -> impl IntoElement {
    let rows = snapshot.peers.peers.iter().map(|peer| {
        DataRow::new(
            peer.addr.clone(),
            [
                peer.addr.clone(),
                peer.state.clone(),
                format::bytes(peer.fetched_bytes),
                format::bytes(peer.uploaded_bytes),
            ],
        )
    });
    DataTable::new()
        .columns([
            DataColumn::new("addr", "Address").width(200.0),
            DataColumn::new("state", "State").width(90.0),
            DataColumn::new("down", "Downloaded").width(110.0),
            DataColumn::new("up", "Uploaded").width(110.0),
        ])
        .rows(rows)
        .empty_message(empty_state_message("peers"))
        .max_body_height(240.0)
}

fn trackers_table(snapshot: &DetailSnapshot) -> impl IntoElement {
    let rows = snapshot.trackers.trackers.iter().map(|tracker| {
        DataRow::new(
            tracker.url.clone(),
            [
                tracker.url.clone(),
                format!("{:?}", tracker.state),
                tracker.seeders.map_or_else(String::new, |n| n.to_string()),
                tracker.leechers.map_or_else(String::new, |n| n.to_string()),
            ],
        )
    });
    DataTable::new()
        .columns([
            DataColumn::new("url", "Tracker").width(260.0),
            DataColumn::new("state", "State").width(90.0),
            DataColumn::new("seeders", "Seeders").width(80.0),
            DataColumn::new("leechers", "Leechers").width(80.0),
        ])
        .rows(rows)
        .empty_message(empty_state_message("trackers"))
        .max_body_height(240.0)
}

impl Render for DetailPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut pane = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .min_h(px(200.0))
            .child(self.tabs.clone());
        if let Some(message) = &self.error {
            pane = pane.child(MessageBar::new(message.clone()).intent(MessageIntent::Error));
        }
        pane.child(self.body(cx))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use gpui::TestAppContext;
    use tokio::runtime::Runtime;
    use wiremock::matchers::{body_string, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../rtbit-client/tests/fixtures"
    );

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap()
    }

    /// A wiremock daemon serving one torrent's detail routes, with the bridge
    /// logged in and installed. Keep the returned runtime alive with the server.
    fn daemon(cx: &mut TestAppContext) -> (Arc<Runtime>, MockServer) {
        // Replies wake gpui tasks from tokio threads, which is real (not simulated) concurrency.
        cx.executor().allow_parking();
        let scratch = Bridge::new("http://127.0.0.1:1").unwrap();
        let runtime = scratch.runtime().clone();
        let server = runtime.block_on(async {
            let server = MockServer::start().await;
            let json =
                |body: String| ResponseTemplate::new(200).set_body_raw(body, "application/json");
            Mock::given(method("POST"))
                .and(path("/auth/login"))
                .respond_with(json(fixture("login.json")))
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/torrents/7"))
                .respond_with(json(fixture("torrent.json")))
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/torrents/7/trackers"))
                .respond_with(json(fixture("trackers.json")))
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/torrents/7/peer_stats"))
                .respond_with(json(fixture("peers.json")))
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/torrents/7/haves"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_bytes(vec![0b1000_0000u8])
                        .insert_header("x-bitfield-len", "10"),
                )
                .mount(&server)
                .await;
            server
        });
        let bridge = Bridge::new(&server.uri()).unwrap();
        let login = bridge.call(|client| async move {
            let mut client = client.lock().await;
            client.login("admin", "secret").await
        });
        runtime.block_on(login).unwrap().unwrap();
        cx.update(|cx| bridge.install(cx));
        (runtime, server)
    }

    /// Drive gpui until `done` holds. Replies arrive from tokio threads, so
    /// park, wait briefly in real time, and re-run.
    fn drive(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
        for _ in 0..500 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("bridge reply never arrived");
    }

    #[test]
    fn tabs_match_the_old_client_after_general() {
        let labels: Vec<&str> = TABS.iter().map(|(_, label)| *label).collect();
        assert_eq!(labels, ["General", "Files", "Peers", "Trackers", "Pieces"]);
    }

    #[test]
    fn every_detail_tab_has_the_expected_named_empty_state() {
        assert_eq!(empty_state_message("general"), "No torrent selected.");
        assert_eq!(
            empty_state_message("files"),
            "Torrent metadata is still resolving."
        );
        assert_eq!(empty_state_message("peers"), "No connected peers.");
        assert_eq!(empty_state_message("trackers"), "No trackers (DHT-only).");
        assert_eq!(
            empty_state_message("pieces"),
            "Torrent metadata is still resolving."
        );
    }

    #[gpui::test]
    fn pieces_tab_shows_held_out_of_total(cx: &mut TestAppContext) {
        let (_runtime, _server) = daemon(cx);
        let pane = cx.new(DetailPane::new);
        pane.update(cx, |pane, cx| pane.select_torrent(7, cx));
        drive(cx, |cx| pane.read_with(cx, |p, _| p.snapshot().is_some()));
        pane.update(cx, |pane, cx| {
            pane.tabs.update(cx, |strip, _| strip.active = 4);
            assert_eq!(pane.active_tab(cx), "pieces");
        });
        pane.read_with(cx, |pane, _| {
            let snapshot = pane.snapshot().unwrap();
            assert_eq!(snapshot.pieces.have.iter().filter(|h| **h).count(), 1);
        });
    }

    #[gpui::test]
    fn selecting_a_torrent_loads_its_detail(cx: &mut TestAppContext) {
        let (_runtime, _server) = daemon(cx);
        let pane = cx.new(DetailPane::new);
        pane.update(cx, |pane, cx| pane.select_torrent(7, cx));
        drive(cx, |cx| pane.read_with(cx, |p, _| p.snapshot().is_some()));
        pane.read_with(cx, |pane, _| {
            let snapshot = pane.snapshot().unwrap();
            assert_eq!(snapshot.torrent.name.as_deref(), Some("example"));
            assert_eq!(snapshot.torrent.files.as_ref().map(Vec::len), Some(2));
            assert_eq!(snapshot.trackers.trackers.len(), 1);
            assert_eq!(snapshot.peers.peers.len(), 1);
            assert_eq!(snapshot.pieces.have.iter().filter(|h| **h).count(), 1);
            assert_eq!(pane.torrent_id(), Some(7));
        });
    }

    #[gpui::test]
    fn file_toggle_posts_the_new_selection(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        // skip.bin (index 1) is excluded in the fixture, so including it posts
        // {"included":true} to files/1.
        runtime.block_on(
            Mock::given(method("POST"))
                .and(path("/torrents/7/files/1/included"))
                .and(body_string(r#"{"included":true}"#))
                .respond_with(ResponseTemplate::new(200))
                .expect(1)
                .mount(&server),
        );
        let pane = cx.new(DetailPane::new);
        pane.update(cx, |pane, cx| pane.select_torrent(7, cx));
        drive(cx, |cx| pane.read_with(cx, |p, _| p.snapshot().is_some()));
        pane.update(cx, |pane, cx| pane.toggle_file(1, cx));
        // Wait for the daemon to see the post; the reload that follows is not needed here.
        let count_posts = |runtime: &Runtime, server: &MockServer| {
            runtime
                .block_on(server.received_requests())
                .unwrap()
                .iter()
                .filter(|r| {
                    r.method.as_str() == "POST" && r.url.path() == "/torrents/7/files/1/included"
                })
                .count()
        };
        drive(cx, |_| count_posts(&runtime, &server) == 1);
        assert_eq!(
            count_posts(&runtime, &server),
            1,
            "exactly one selection post reaches the daemon"
        );
        pane.read_with(cx, |p, _| {
            assert!(p.error().is_none(), "post must not fail")
        });
    }

    fn file(path: &str, included: bool) -> TorrentFile {
        TorrentFile {
            name: path.to_owned(),
            components: path.split('/').map(str::to_owned).collect(),
            length: 10,
            included,
        }
    }

    fn sample() -> Vec<TorrentFile> {
        vec![
            file("show/s1/e1.mkv", true),
            file("show/s1/e2.mkv", false),
            file("show/notes.txt", true),
            file("readme.txt", true),
        ]
    }

    #[test]
    fn tree_nests_folders_before_files() {
        let tree = file_tree(&sample(), &HashSet::new());
        let ids: Vec<&str> = tree.iter().map(|item| item.id.as_ref()).collect();
        assert_eq!(ids, ["dir:show", "file:3"]);
        let show = &tree[0];
        let ids: Vec<&str> = show.children.iter().map(|item| item.id.as_ref()).collect();
        assert_eq!(ids, ["dir:show/s1", "file:2"]);
        assert_eq!(show.children[0].children.len(), 2);
        assert!(show.badge.as_deref().unwrap().ends_with("Mixed"));
        assert!(
            show.children[0].children[1]
                .badge
                .as_deref()
                .unwrap()
                .ends_with("Do not download")
        );
    }

    #[test]
    fn collapsed_folders_stay_collapsed() {
        let collapsed: HashSet<SharedString> = [dir_id("show")].into_iter().collect();
        let tree = file_tree(&sample(), &collapsed);
        assert!(!tree[0].expanded);
        assert!(tree[0].children[0].expanded);
    }

    #[test]
    fn folder_covers_every_file_below_it() {
        let files = sample();
        assert_eq!(indices_under(&files, "dir:show"), [0, 1, 2]);
        assert_eq!(indices_under(&files, "dir:show/s1"), [0, 1]);
        assert_eq!(indices_under(&files, "file:3"), [3]);
        assert!(indices_under(&files, "file:9").is_empty());
        assert_eq!(inclusion(&files, &[0, 1]), CheckboxState::Indeterminate);
        assert_eq!(inclusion(&files, &[0, 2]), CheckboxState::Checked);
        assert_eq!(inclusion(&files, &[1]), CheckboxState::Unchecked);
    }

    #[test]
    fn new_selection_sets_only_the_changed_files() {
        let files = sample();
        assert_eq!(new_selection(&files, &[0, 1], true), [0, 1, 2, 3]);
        assert_eq!(new_selection(&files, &[0, 1, 2], false), [3]);
    }

    #[gpui::test]
    fn skipping_a_folder_posts_the_whole_selection(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        // Both fixture files sit at the root; skipping both leaves nothing selected.
        runtime.block_on(
            Mock::given(method("POST"))
                .and(path("/torrents/7/update_only_files"))
                .and(body_string(r#"{"only_files":[]}"#))
                .respond_with(ResponseTemplate::new(200))
                .expect(1)
                .mount(&server),
        );
        let pane = cx.new(DetailPane::new);
        pane.update(cx, |pane, cx| pane.select_torrent(7, cx));
        drive(cx, |cx| pane.read_with(cx, |p, _| p.snapshot().is_some()));
        pane.update(cx, |pane, cx| pane.set_included(&[0, 1, 0], false, cx));
        let count_posts = |runtime: &Runtime, server: &MockServer| {
            runtime
                .block_on(server.received_requests())
                .unwrap()
                .iter()
                .filter(|r| r.url.path() == "/torrents/7/update_only_files")
                .count()
        };
        drive(cx, |_| count_posts(&runtime, &server) == 1);
        pane.read_with(cx, |p, _| assert!(p.error().is_none()));
    }
}
