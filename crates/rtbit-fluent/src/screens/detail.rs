//! Torrent detail pane: a bottom pane with a `TabStrip` (General, Files,
//! Peers, Trackers) for the selected torrent.
//!
//! Every read and write runs on the bridge's tokio runtime. The pane keeps
//! the last good [`DetailSnapshot`] in an [`AsyncResource`], so a reply from a
//! superseded selection cannot overwrite the current one. A file checkbox posts
//! the whole new selection with `update_only_files`, then reloads the detail.

use fluent_core::AsyncResource;
use fluent_layout::{DataColumn, DataRow, DataTable, MessageBar, MessageIntent, TabItem, TabStrip};
use fluent_primitives::{Checkbox, Label};
use gpui::{
    App, AsyncApp, Context, Entity, IntoElement, Render, Task, WeakEntity, Window, div, prelude::*,
    px,
};
use rtbit_client::{Error, TorrentFile};
use rtbit_ui_core::{DetailSnapshot, format, map_error};

use crate::bridge::{Bridge, Poller, SharedClient};

/// Tab ids, in strip order.
const TABS: [(&str, &str); 4] = [
    ("general", "General"),
    ("files", "Files"),
    ("peers", "Peers"),
    ("trackers", "Trackers"),
];

/// The detail pane for one selected torrent.
pub struct DetailPane {
    torrent_id: Option<u64>,
    detail: AsyncResource<DetailSnapshot, String>,
    tabs: Entity<TabStrip>,
    error: Option<String>,
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

/// The selection a file post needs: the current included set with `index`
/// set to `included`, sorted.
#[must_use]
pub fn next_selection(files: &[TorrentFile], index: usize, included: bool) -> Vec<usize> {
    let mut indices: Vec<usize> = files
        .iter()
        .enumerate()
        .filter_map(|(i, file)| (file.included && i != index).then_some(i))
        .collect();
    if included {
        indices.push(index);
        indices.sort_unstable();
    }
    indices
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
        let indices = next_selection(&files, index, !file.included);
        let rx = cx.global::<Bridge>().call(move |client| async move {
            let mut client = client.lock().await;
            client.update_only_files(id, &indices).await
        });
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
                "Select a torrent to see its details."
            };
            return Label::new(text).into_any_element();
        };
        match self.active_tab(cx) {
            "files" => files(snapshot, cx).into_any_element(),
            "peers" => peers_table(snapshot).into_any_element(),
            "trackers" => trackers_table(snapshot).into_any_element(),
            _ => general(snapshot).into_any_element(),
        }
    }
}

/// One checkbox per file. Checking a box posts the new selection via `toggle_file`.
fn files(snapshot: &DetailSnapshot, cx: &mut Context<DetailPane>) -> impl IntoElement {
    let me = cx.entity();
    let rows = snapshot
        .torrent
        .files
        .iter()
        .flatten()
        .enumerate()
        .map(|(index, file)| {
            let me = me.clone();
            let checkbox = Checkbox::new(("torrent-file", index))
                .checked(file.included)
                .on_click(move |_, _, _, cx| {
                    me.update(cx, |pane, cx| pane.toggle_file(index, cx));
                });
            div()
                .flex()
                .gap_2()
                .child(checkbox)
                .child(Label::new(format!(
                    "{}  {}",
                    file.name,
                    format::bytes(file.length)
                )))
        });
    div().flex().flex_col().gap_1().children(rows)
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
        .empty_message("No live peers")
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
        .empty_message("No trackers")
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

    fn file(name: &str, included: bool) -> TorrentFile {
        TorrentFile {
            name: name.to_owned(),
            components: vec![name.to_owned()],
            length: 1,
            included,
        }
    }

    #[test]
    fn next_selection_adds_and_removes_one_index() {
        let files = [file("a", true), file("b", false), file("c", true)];
        // Include b: the current selection plus index 1, sorted.
        assert_eq!(next_selection(&files, 1, true), vec![0, 1, 2]);
        // Exclude a: the current selection without index 0.
        assert_eq!(next_selection(&files, 0, false), vec![2]);
        // Excluding a file that is not selected leaves the rest alone.
        assert_eq!(next_selection(&files, 1, false), vec![0, 2]);
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
        // skip.bin (index 1) is excluded in the fixture, so including it posts [0, 1].
        runtime.block_on(
            Mock::given(method("POST"))
                .and(path("/torrents/7/update_only_files"))
                .and(body_string(r#"{"only_files":[0,1]}"#))
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
                    r.method.as_str() == "POST" && r.url.path() == "/torrents/7/update_only_files"
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
}
