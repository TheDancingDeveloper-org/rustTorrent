//! RSS screen: the feed item list, a Download action for the selected item,
//! an empty state, and an error bar when a call fails.
//!
//! Loads and downloads run on the bridge's tokio runtime. The UI thread only
//! marks the request pending and awaits the reply in a gpui task.

use fluent_core::AsyncResource;
use fluent_layout::{DataColumn, DataRow, DataTable, MessageBar, MessageIntent};
use fluent_primitives::{Button, ButtonAppearance, Label};
use gpui::{
    AsyncApp, ClickEvent, Context, Entity, IntoElement, Render, SharedString, Task, WeakEntity,
    Window, div, prelude::*,
};
use rtbit_client::{Error, RssItem, RssSnapshot};
use rtbit_ui_core::map_error;

use crate::bridge::Bridge;

/// The RSS screen. The shell builds one when the RSS nav entry opens.
pub struct RssScreen {
    snapshot: AsyncResource<RssSnapshot, String>,
    /// The item the user picked in the table, if it is still listed.
    selected: Option<String>,
    /// Item id whose download is in flight, so one item is not queued twice.
    downloading: Option<String>,
    error: Option<String>,
    _load: Option<Task<()>>,
    _download: Option<Task<()>>,
}

impl RssScreen {
    #[must_use]
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: AsyncResource::default(),
            selected: None,
            downloading: None,
            error: None,
            _load: None,
            _download: None,
        }
    }

    /// The feeds, items and rules once a load has landed.
    #[must_use]
    pub fn snapshot(&self) -> Option<&RssSnapshot> {
        self.snapshot.ready_value()
    }

    /// The error bar text, if the last call failed.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The item id whose download is in flight, if any.
    #[must_use]
    pub fn downloading(&self) -> Option<&str> {
        self.downloading.as_deref()
    }

    /// The item id the user selected, if it is still in the list.
    #[must_use]
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Pick one item in the table. Ids not in the list are ignored.
    pub fn select(&mut self, id: &str, cx: &mut Context<Self>) {
        let listed = self
            .snapshot()
            .is_some_and(|snapshot| snapshot.items.iter().any(|item| item.id == id));
        if listed {
            self.selected = Some(id.to_owned());
            cx.notify();
        }
    }

    /// Fetch feeds, items and rules from the daemon.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        self._load = Some(Self::start_load(self, cx));
    }

    fn start_load(this: &mut Self, cx: &mut Context<Self>) -> Task<()> {
        Bridge::load(
            this,
            cx,
            |screen| &mut screen.snapshot,
            |client| async move {
                let mut client = client.lock().await;
                client.rss().await
            },
        )
    }

    /// Ask the daemon to add the selected item as a torrent. Ignored while a
    /// download is in flight, when nothing is selected, or when the item
    /// cannot be downloaded.
    pub fn download_selected(&mut self, cx: &mut Context<Self>) {
        if self.downloading.is_some() {
            return;
        }
        let Some(id) = self.selected.clone() else {
            return;
        };
        let downloadable = self
            .snapshot()
            .and_then(|snapshot| snapshot.items.iter().find(|item| item.id == id))
            .is_some_and(can_download);
        if !downloadable {
            return;
        }
        self.downloading = Some(id.clone());
        self.error = None;
        cx.notify();
        let rx = cx.global::<Bridge>().call(move |client| async move {
            let mut client = client.lock().await;
            client.download_rss_item(&id).await
        });
        self._download = Some(cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let reply = match rx.await {
                Ok(reply) => reply,
                Err(_) => Err(Error::Unreachable("request cancelled".to_owned())),
            };
            let _ = this.update(cx, |this, cx| this.finish_download(reply, cx));
        }));
    }

    fn finish_download(&mut self, reply: Result<(), Error>, cx: &mut Context<Self>) {
        self.downloading = None;
        match reply {
            // The daemon marks the item downloaded; reload to show it.
            Ok(()) => self._load = Some(Self::start_load(self, cx)),
            Err(err) => self.error = Some(map_error(&err)),
        }
        cx.notify();
    }
}

/// Whether an item can be downloaded now: it has a link and is not already
/// downloaded.
#[must_use]
pub fn can_download(item: &RssItem) -> bool {
    !item.downloaded && item.url.is_some()
}

/// The status column text for an item.
#[must_use]
pub fn item_status(item: &RssItem) -> &'static str {
    if item.downloaded {
        "Downloaded"
    } else if item.url.is_none() {
        "No link"
    } else {
        "Ready"
    }
}

impl Render for RssScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let me: Entity<Self> = cx.entity();
        let mut root = div().flex().flex_col().gap_3().child(Label::new("RSS"));

        if let Some(message) = self.error.clone() {
            root = root.child(MessageBar::new(message).intent(MessageIntent::Error));
        }

        let table = match self.snapshot.ready_value() {
            None if self.snapshot.is_loading() => DataTable::new().loading("Loading RSS…"),
            None => DataTable::new().error_message(
                self.snapshot
                    .error()
                    .cloned()
                    .unwrap_or_else(|| "RSS is not loaded.".to_owned()),
            ),
            Some(snapshot) if snapshot.items.is_empty() => {
                DataTable::new().empty_message("No RSS items yet.")
            }
            Some(snapshot) => {
                let rows = snapshot.items.iter().map(|item| {
                    DataRow::new(
                        item.id.clone(),
                        [
                            item.title.clone(),
                            item.feed_name.clone(),
                            item_status(item).to_owned(),
                        ],
                    )
                });
                let table = DataTable::new()
                    .columns([
                        DataColumn::new("title", "Title").width(360.0),
                        DataColumn::new("feed", "Feed").width(160.0),
                        DataColumn::new("status", "Status").width(120.0),
                    ])
                    .rows(rows)
                    .on_row_select(move |id: SharedString, _window, cx| {
                        me.update(cx, |screen, cx| screen.select(&id, cx));
                    });
                match self.selected.as_deref() {
                    Some(id) => table.selected(id.to_owned()),
                    None => table,
                }
            }
        };
        root = root.child(table);

        let can_act = self.downloading.is_none()
            && self
                .selected
                .as_deref()
                .and_then(|id| self.snapshot()?.items.iter().find(|item| item.id == id))
                .is_some_and(can_download);
        let label = if self.downloading.is_some() {
            "Downloading…"
        } else {
            "Download"
        };
        let click = cx.listener(|this, _: &ClickEvent, _window, cx| this.download_selected(cx));
        root.child(
            Button::new("rss-download")
                .label(label)
                .appearance(ButtonAppearance::Accent)
                .disabled(!can_act)
                .on_click(click),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../rtbit-client/tests/fixtures"
    );

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap()
    }

    const ITEMS: &str = r#"[
        {"id":"a","feed_name":"Show","title":"Ready one","url":"magnet:?xt=urn:btih:aa","downloaded":false},
        {"id":"b","feed_name":"Show","title":"Done one","url":"magnet:?xt=urn:btih:bb","downloaded":true},
        {"id":"c","feed_name":"Show","title":"No link","url":null,"downloaded":false}
    ]"#;

    /// Start a wiremock daemon on the bridge runtime and install the bridge
    /// with an authenticated client. `items` is the `/rss/items` body.
    fn daemon(
        cx: &mut TestAppContext,
        items: &str,
    ) -> (std::sync::Arc<tokio::runtime::Runtime>, MockServer) {
        cx.executor().allow_parking();
        let scratch = Bridge::new("http://127.0.0.1:1").unwrap();
        let runtime = scratch.runtime().clone();
        let items = items.to_owned();
        let server = runtime.block_on(async {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/auth/login"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(fixture("login.json"), "application/json"),
                )
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/rss/feeds"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(fixture("rss-feeds.json"), "application/json"),
                )
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/rss/items"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_raw(items, "application/json"),
                )
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/rss/rules"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(fixture("rss-rules.json"), "application/json"),
                )
                .mount(&server)
                .await;
            server
        });
        let client = runtime.block_on(async {
            let mut client = rtbit_client::Client::new(server.uri()).unwrap();
            client.login("admin", "secret").await.unwrap();
            client
        });
        let bridge = Bridge::from_parts(runtime.clone(), client);
        cx.update(|cx| bridge.install(cx));
        (runtime, server)
    }

    /// Drive gpui until `done` holds. Replies arrive from tokio threads.
    fn drive(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
        for _ in 0..500 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("rss reply never arrived");
    }

    fn loaded(cx: &mut TestAppContext) -> gpui::Entity<RssScreen> {
        let screen = cx.new(RssScreen::new);
        screen.update(cx, |screen, cx| screen.load(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.snapshot.is_loading()));
        screen
    }

    #[gpui::test]
    fn load_lists_feeds_items_and_rules(cx: &mut TestAppContext) {
        let (_runtime, _server) = daemon(cx, ITEMS);
        let screen = loaded(cx);
        screen.read_with(cx, |screen, _| {
            let snapshot = screen.snapshot().expect("snapshot ready");
            assert_eq!(snapshot.items.len(), 3);
            assert_eq!(snapshot.feeds.len(), 1);
            assert_eq!(snapshot.rules.len(), 1);
        });
    }

    #[gpui::test]
    fn empty_items_are_a_ready_empty_list(cx: &mut TestAppContext) {
        let (_runtime, _server) = daemon(cx, "[]");
        let screen = loaded(cx);
        screen.read_with(cx, |screen, _| {
            assert!(screen.snapshot().expect("ready").items.is_empty());
            assert!(screen.error().is_none());
        });
    }

    #[gpui::test]
    fn only_listed_items_are_selectable(cx: &mut TestAppContext) {
        let (_runtime, _server) = daemon(cx, ITEMS);
        let screen = loaded(cx);
        screen.update(cx, |screen, cx| {
            screen.select("missing", cx);
            assert!(screen.selected().is_none());
            screen.select("a", cx);
        });
        screen.read_with(cx, |screen, _| assert_eq!(screen.selected(), Some("a")));
    }

    #[gpui::test]
    fn downloaded_and_linkless_items_cannot_download(cx: &mut TestAppContext) {
        let (_runtime, _server) = daemon(cx, ITEMS);
        let screen = loaded(cx);
        screen.read_with(cx, |screen, _| {
            let items = &screen.snapshot().unwrap().items;
            assert!(can_download(&items[0]));
            assert!(!can_download(&items[1]), "already downloaded");
            assert!(!can_download(&items[2]), "no link");
            assert_eq!(item_status(&items[1]), "Downloaded");
            assert_eq!(item_status(&items[2]), "No link");
        });
    }

    #[gpui::test]
    fn download_posts_the_item_and_reloads(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx, ITEMS);
        runtime.block_on(async {
            Mock::given(method("POST"))
                .and(path("/rss/items/a/download"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
        });
        let screen = loaded(cx);
        screen.update(cx, |screen, cx| {
            screen.select("a", cx);
            screen.download_selected(cx);
            assert_eq!(screen.downloading(), Some("a"));
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.downloading().is_none()));
        screen.read_with(cx, |screen, _| {
            assert!(screen.error().is_none(), "download should succeed");
        });
    }

    #[gpui::test]
    fn refused_download_shows_an_error(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx, ITEMS);
        runtime.block_on(async {
            Mock::given(method("POST"))
                .and(path("/rss/items/a/download"))
                .respond_with(ResponseTemplate::new(500))
                .mount(&server)
                .await;
        });
        let screen = loaded(cx);
        screen.update(cx, |screen, cx| {
            screen.select("a", cx);
            screen.download_selected(cx);
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.downloading().is_none()));
        screen.read_with(cx, |screen, _| {
            assert!(screen.error().is_some(), "error bar must show the refusal");
        });
    }
}
