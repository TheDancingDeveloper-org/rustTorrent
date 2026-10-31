//! Async bridge between the tokio-based `rtbit-client` and gpui.
//!
//! The earlier immediate-mode shell called `block_on` on a current-thread runtime every frame.
//! gpui has its own executor and the main thread must never block, so the
//! bridge owns a multi-thread tokio runtime in a gpui [`Global`]. Client calls
//! run there; each result comes back over a oneshot channel that a gpui task
//! awaits, and lands in a fluent-core [`AsyncResource`]. The resource's
//! generation token drops replies that a newer request has superseded.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use fluent_core::AsyncResource;
use gpui::{App, AsyncApp, Context, Global, Task, WeakEntity};
use rtbit_client::{Client, Error};
use rtbit_ui_core::Snapshot;
use tokio::runtime::Runtime;
use tokio::sync::{Mutex, oneshot};

/// How often [`Bridge::poll_snapshot`] refreshes the main screen.
pub const SNAPSHOT_INTERVAL: Duration = Duration::from_secs(1);

/// Worker threads for the bridge runtime. Calls are I/O bound.
const WORKER_THREADS: usize = 2;

/// The client shared by every call; tokio's mutex serialises token refreshes.
pub type SharedClient = Arc<Mutex<Client>>;

/// Owns the tokio runtime and the daemon client. Install with [`Bridge::install`].
pub struct Bridge {
    runtime: Arc<Runtime>,
    client: SharedClient,
}

impl Global for Bridge {}

impl Bridge {
    /// Build the runtime and a client for `base_url`. Does not touch the network.
    pub fn new(base_url: &str) -> Result<Self, Error> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(WORKER_THREADS)
            .thread_name("rtbit-fluent-io")
            .enable_all()
            .build()
            .map_err(|err| Error::Unreachable(err.to_string()))?;
        // reqwest picks up the ambient runtime lazily, but build inside it to be safe.
        let client = {
            let _guard = runtime.enter();
            Client::new(base_url)?
        };
        Ok(Self {
            runtime: Arc::new(runtime),
            client: Arc::new(Mutex::new(client)),
        })
    }

    /// Make the bridge reachable from any gpui context as a global.
    pub fn install(self, cx: &mut App) {
        cx.set_global(self);
    }

    /// Build a bridge from a runtime and an already-logged-in client. Tests
    /// use this to point screens at a mock daemon.
    #[cfg(test)]
    pub fn from_parts(runtime: Arc<Runtime>, client: Client) -> Self {
        Self {
            runtime,
            client: Arc::new(Mutex::new(client)),
        }
    }

    /// The runtime, for callers that need to spawn their own I/O.
    pub fn runtime(&self) -> &Arc<Runtime> {
        &self.runtime
    }

    /// Run `call` on the tokio runtime. The receiver is executor-agnostic, so
    /// a gpui task can await it without blocking the UI thread. It errors only
    /// if the runtime dropped the call (shutdown). Dropping the receiver
    /// aborts the call on the runtime.
    pub fn call<T, F, Fut>(&self, call: F) -> oneshot::Receiver<Result<T, Error>>
    where
        T: Send + 'static,
        F: FnOnce(SharedClient) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, Error>> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        let client = self.client.clone();
        self.runtime.spawn(async move {
            let mut tx = tx;
            tokio::select! {
                reply = call(client) => {
                    // The receiver may be gone (view dropped); nothing to do then.
                    let _ = tx.send(reply);
                }
                () = tx.closed() => {}
            }
        });
        rx
    }

    /// Start a load into the resource `select` picks out of the view `this`.
    /// The returned task delivers the reply; drop it to cancel delivery.
    pub fn load<V, T, F, Fut>(
        this: &mut V,
        cx: &mut Context<V>,
        select: fn(&mut V) -> &mut AsyncResource<T, String>,
        call: F,
    ) -> Task<()>
    where
        V: 'static,
        T: Send + 'static,
        F: FnOnce(SharedClient) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, Error>> + Send + 'static,
    {
        let request = select(this).begin_load();
        cx.notify();
        let rx = cx.global::<Bridge>().call(call);
        cx.spawn(async move |view: WeakEntity<V>, cx: &mut AsyncApp| {
            let reply = await_reply(rx).await;
            let _ = view.update(cx, |view, cx| {
                let resource = select(view);
                let accepted = match reply {
                    Ok(value) => resource.resolve(request, value),
                    Err(err) => resource.reject(request, err),
                };
                if accepted {
                    cx.notify();
                }
            });
        })
    }

    /// Poll the main-screen [`Snapshot`] every [`SNAPSHOT_INTERVAL`] into the
    /// resource `select` picks out. Polling stops when the returned
    /// [`Poller`] is dropped or the view is released.
    pub fn poll_snapshot<V: 'static>(
        cx: &mut Context<V>,
        select: fn(&mut V) -> &mut AsyncResource<Snapshot, String>,
    ) -> Poller {
        let task = cx.spawn(async move |view: WeakEntity<V>, cx: &mut AsyncApp| {
            loop {
                let Ok(pending) = view.update(cx, |view, cx| {
                    let request = select(view).begin_load();
                    cx.notify();
                    (request, cx.global::<Bridge>().call(fetch_snapshot))
                }) else {
                    return;
                };
                let (request, rx) = pending;
                let reply = await_reply(rx).await;
                let updated = view.update(cx, |view, cx| {
                    let resource = select(view);
                    let accepted = match reply {
                        Ok(value) => resource.resolve(request, value),
                        Err(err) => resource.reject(request, err),
                    };
                    if accepted {
                        cx.notify();
                    }
                });
                if updated.is_err() {
                    return;
                }
                cx.background_executor().timer(SNAPSHOT_INTERVAL).await;
            }
        });
        Poller { _task: task }
    }
}

/// A running snapshot poll. Dropping it cancels the poll.
#[must_use = "dropping the poller cancels the poll"]
pub struct Poller {
    _task: Task<()>,
}

/// One poll of the main screen: the torrent list with stats, then session speeds.
pub async fn fetch_snapshot(client: SharedClient) -> Result<Snapshot, Error> {
    let mut client = client.lock().await;
    let torrents = client.torrents_with_stats().await?;
    let stats = client.stats().await?;
    Ok(Snapshot { torrents, stats })
}

async fn await_reply<T>(rx: oneshot::Receiver<Result<T, Error>>) -> Result<T, String> {
    match rx.await {
        Ok(reply) => reply.map_err(|err| err.to_string()),
        Err(_) => Err("request cancelled".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, TestAppContext};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../rtbit-client/tests/fixtures"
    );

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap()
    }

    #[derive(Default)]
    struct Screen {
        snapshot: AsyncResource<Snapshot, String>,
        _poller: Option<Poller>,
        _load: Option<Task<()>>,
    }

    fn snapshot_of(screen: &mut Screen) -> &mut AsyncResource<Snapshot, String> {
        &mut screen.snapshot
    }

    /// Start a wiremock daemon on the bridge runtime and install the bridge.
    fn daemon(cx: &mut TestAppContext) -> (Arc<Runtime>, MockServer) {
        // Replies wake gpui tasks from tokio threads, which is real (not simulated) concurrency.
        cx.executor().allow_parking();
        // The bridge needs a URL before the server exists; start it on a scratch bridge's runtime.
        let scratch = Bridge::new("http://127.0.0.1:1").unwrap();
        let runtime = scratch.runtime().clone();
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
                .and(path("/torrents"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(fixture("torrents.json"), "application/json"),
                )
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/stats"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(fixture("stats.json"), "application/json"),
                )
                .mount(&server)
                .await;
            server
        });
        let client = runtime.block_on(async {
            let mut client = Client::new(server.uri()).unwrap();
            client.login("admin", "secret").await.unwrap();
            client
        });
        let bridge = Bridge {
            runtime: runtime.clone(),
            client: Arc::new(Mutex::new(client)),
        };
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
    fn load_resolves_snapshot_from_daemon(cx: &mut TestAppContext) {
        let (_runtime, _server) = daemon(cx);
        let screen = cx.new(|_| Screen::default());
        screen.update(cx, |screen, cx| {
            let task = Bridge::load(screen, cx, snapshot_of, fetch_snapshot);
            screen._load = Some(task);
            assert!(screen.snapshot.is_loading());
        });
        drive(cx, |cx| {
            screen.read_with(cx, |s, _| !s.snapshot.is_loading())
        });
        screen.read_with(cx, |screen, _| {
            let snapshot = screen
                .snapshot
                .ready_value()
                .unwrap_or_else(|| panic!("snapshot not ready: {:?}", screen.snapshot.error()));
            assert_eq!(
                snapshot.torrents.torrents[0].name.as_deref(),
                Some("example")
            );
        });
    }

    #[gpui::test]
    fn stale_reply_is_dropped(cx: &mut TestAppContext) {
        let (_runtime, _server) = daemon(cx);
        let screen = cx.new(|_| Screen::default());
        screen.update(cx, |screen, cx| {
            // The first call fails, but a newer request supersedes it before it lands.
            let stale = Bridge::load(screen, cx, snapshot_of, |_| async {
                Err(Error::Unreachable("stale".into()))
            });
            stale.detach();
            let task = Bridge::load(screen, cx, snapshot_of, fetch_snapshot);
            screen._load = Some(task);
        });
        drive(cx, |cx| {
            screen.read_with(cx, |s, _| !s.snapshot.is_loading())
        });
        screen.read_with(cx, |screen, _| {
            assert!(screen.snapshot.is_ready(), "stale error must not win");
            assert_eq!(screen.snapshot.generation(), 2);
        });
    }

    #[gpui::test]
    fn poller_refreshes_and_stops_on_drop(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        let screen = cx.new(|_| Screen::default());
        screen.update(cx, |screen, cx| {
            screen._poller = Some(Bridge::poll_snapshot(cx, snapshot_of));
        });
        // Step the fake clock past each 1s timer until three polls have started.
        drive(cx, |cx| {
            cx.executor().advance_clock(SNAPSHOT_INTERVAL);
            screen.read_with(cx, |s, _| s.snapshot.generation() >= 3)
        });
        screen.update(cx, |screen, _| screen._poller = None);
        cx.run_until_parked();
        // Let the runtime observe the dropped receiver and abort any call in flight.
        std::thread::sleep(Duration::from_millis(50));
        let before = runtime.block_on(server.received_requests()).unwrap().len();
        let generation = screen.read_with(cx, |s, _| s.snapshot.generation());
        for _ in 0..5 {
            cx.executor().advance_clock(SNAPSHOT_INTERVAL * 2);
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(10));
        }
        let after = runtime.block_on(server.received_requests()).unwrap().len();
        assert_eq!(before, after, "no requests after the poller is dropped");
        assert_eq!(
            generation,
            screen.read_with(cx, |s, _| s.snapshot.generation())
        );
    }
}
