//! Screen state the shells share, and the daemon session behind a connect.
//! Calls block; a shell runs them off its UI thread.

use rtbit_client::{
    AuthStatus, Client, Error, PeerList, PieceProgress, RssRuleBody, RssRuleEpisode, RssSnapshot,
    SessionStats, SettingsSnapshot, Torrent, TorrentList, TrackerList,
};

use crate::actions::{AddRequest, AddSource, DetailAction, RowAction, SettingsChanges};

/// Which screen is showing. Later items add setup, detail, add and settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    /// Daemon URL, username and password.
    Connect,
    /// First-run account, only while the daemon says setup is still open.
    Setup,
    /// Placeholder until the torrent grid lands.
    Main,
    /// Read/write preferences: General, Connection, Speed, Folders, Seeding.
    Settings,
    /// Feeds and items, read-only, plus download one item.
    Rss,
}

/// What the status bar says about the daemon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

impl Connection {
    /// What the status bar says.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Disconnected => "Disconnected",
            Self::Connecting => "Connecting",
            Self::Connected => "Connected",
            Self::Error => "Error",
        }
    }
}

/// One poll of the main screen: the torrent list (with stats) and session speeds.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub torrents: TorrentList,
    pub stats: SessionStats,
}

/// How a connect click reaches the daemon. Tests substitute a value that
/// never opens a socket. `snapshot` is what the 2-second poll calls.
///
/// A session is opened once by `login` and then reused. `snapshot` and `act`
/// must not log in again; the client refreshes its own token on a 401.
pub trait LoginAttempt: Send {
    /// Try the credentials and keep the session. The password must not be written to a log.
    /// Returns the refresh token so the shell can put it in the keychain.
    fn login(&self, url: &str, username: &str, password: &str) -> Result<Option<String>, Error>;

    /// Continue a previous session from a refresh token. `Err` means the token
    /// is no longer accepted, and the shell asks for the password again.
    fn resume(&self, url: &str, refresh_token: &str) -> Result<Option<String>, Error> {
        let _ = (url, refresh_token);
        Err(Error::Unauthorised)
    }

    /// Drop the live session. The shell clears the keychain itself.
    fn logout(&self) {}

    /// Whether first-run setup is still open. `Err` means the daemon could
    /// not be asked, and the shell stays on the connect screen.
    fn status(&self, url: &str) -> Result<AuthStatus, Error> {
        let _ = url;
        Ok(AuthStatus {
            auth_enabled: true,
            setup_required: false,
        })
    }

    /// Create the first account. Returns the refresh token, as `login` does.
    /// The setup token and password must not be written to a log.
    fn setup(
        &self,
        url: &str,
        username: &str,
        password: &str,
        setup_token: &str,
    ) -> Result<Option<String>, Error> {
        let _ = (url, username, password, setup_token);
        Err(Error::NotSetUp)
    }

    /// The refresh token the live session holds now. After a 401 the client
    /// rotates it; the shell writes the new one back to the keychain.
    fn rotated_refresh(&self) -> Option<String> {
        None
    }

    /// The current list and session speeds, on the session `login` opened.
    fn snapshot(&self) -> Result<Snapshot, Error> {
        Err(Error::Unreachable("no snapshot".into()))
    }

    /// Start, pause or delete one torrent on the session `login` opened.
    fn act(&self, id: u64, action: RowAction) -> Result<(), Error> {
        let _ = (id, action);
        Err(Error::Unreachable("no action".into()))
    }

    /// Add a magnet or a torrent file on the session `login` opened.
    fn add(&self, request: &AddRequest) -> Result<(), Error> {
        let _ = request;
        Err(Error::Unreachable("no add".into()))
    }

    /// Files, trackers, live peers and piece progress for one torrent.
    fn detail(&self, id: u64) -> Result<DetailSnapshot, Error> {
        let _ = id;
        Err(Error::Unreachable("no detail".into()))
    }

    /// Change the file selection or attach a tracker.
    fn detail_act(&self, id: u64, action: &DetailAction) -> Result<(), Error> {
        let _ = (id, action);
        Err(Error::Unreachable("no detail action".into()))
    }

    /// Read the five settings tabs.
    fn load_settings(&self) -> Result<SettingsSnapshot, Error> {
        Err(Error::Unreachable("no settings".into()))
    }

    /// Post the sections the user changed. The daemon validates caps and the filter.
    fn save_settings(&self, changes: &SettingsChanges) -> Result<(), Error> {
        let _ = changes;
        Err(Error::Unreachable("no settings".into()))
    }

    /// Feeds, items and auto-download rules.
    fn load_rss(&self) -> Result<RssSnapshot, Error> {
        Err(Error::Unreachable("no rss".into()))
    }

    /// Ask the daemon to add one item as a torrent.
    fn download_rss(&self, id: &str) -> Result<(), Error> {
        let _ = id;
        Err(Error::Unreachable("no rss".into()))
    }

    /// Add a rule (`id` is `None`) or replace one. A pattern the daemon cannot
    /// compile comes back as a 400 whose message says why.
    fn save_rss_rule(&self, id: Option<&str>, body: &RssRuleBody) -> Result<(), Error> {
        let _ = (id, body);
        Err(Error::Unreachable("no rss".into()))
    }

    /// Delete one rule.
    fn delete_rss_rule(&self, id: &str) -> Result<(), Error> {
        let _ = id;
        Err(Error::Unreachable("no rss".into()))
    }

    /// Episodes one rule matched, newest first.
    fn rss_rule_episodes(&self, id: &str) -> Result<Vec<RssRuleEpisode>, Error> {
        let _ = id;
        Err(Error::Unreachable("no rss".into()))
    }
}

/// One poll of the selected torrent. The list poll does not fetch this.
#[derive(Clone, Debug)]
pub struct DetailSnapshot {
    pub torrent: Torrent,
    pub trackers: TrackerList,
    pub peers: PeerList,
    pub pieces: PieceProgress,
}

/// The real attempt: `rtbit-client` on a private current-thread runtime.
#[derive(Clone, Copy, Debug, Default)]
pub struct DaemonLogin;

/// One runtime and one logged-in client, shared by the poll and the row actions.
struct Session {
    runtime: tokio::runtime::Runtime,
    client: Client,
}

fn sessions() -> &'static std::sync::Mutex<Option<Session>> {
    static SESSION: std::sync::OnceLock<std::sync::Mutex<Option<Session>>> =
        std::sync::OnceLock::new();
    SESSION.get_or_init(|| std::sync::Mutex::new(None))
}

impl LoginAttempt for DaemonLogin {
    fn login(&self, url: &str, username: &str, password: &str) -> Result<Option<String>, Error> {
        let runtime = runtime()?;
        let client = runtime.block_on(async {
            let mut client = Client::new(url)?;
            client.login(username, password).await?;
            Ok::<Client, Error>(client)
        })?;
        let refresh = client.refresh_token().map(str::to_owned);
        *sessions().lock().unwrap_or_else(|err| err.into_inner()) =
            Some(Session { runtime, client });
        Ok(refresh)
    }

    fn resume(&self, url: &str, refresh_token: &str) -> Result<Option<String>, Error> {
        let runtime = runtime()?;
        let client = runtime.block_on(async {
            let mut client = Client::new(url)?;
            client.resume(refresh_token).await?;
            Ok::<Client, Error>(client)
        })?;
        let refresh = client.refresh_token().map(str::to_owned);
        *sessions().lock().unwrap_or_else(|err| err.into_inner()) =
            Some(Session { runtime, client });
        Ok(refresh)
    }

    fn logout(&self) {
        *sessions().lock().unwrap_or_else(|err| err.into_inner()) = None;
    }

    fn status(&self, url: &str) -> Result<AuthStatus, Error> {
        let runtime = runtime()?;
        runtime.block_on(async {
            let client = Client::new(url)?;
            client.auth_status().await
        })
    }

    fn setup(
        &self,
        url: &str,
        username: &str,
        password: &str,
        setup_token: &str,
    ) -> Result<Option<String>, Error> {
        let runtime = runtime()?;
        let client = runtime.block_on(async {
            let mut client = Client::new(url)?;
            client.setup(username, password, setup_token).await?;
            Ok::<Client, Error>(client)
        })?;
        let refresh = client.refresh_token().map(str::to_owned);
        *sessions().lock().unwrap_or_else(|err| err.into_inner()) =
            Some(Session { runtime, client });
        Ok(refresh)
    }

    fn rotated_refresh(&self) -> Option<String> {
        sessions()
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .as_ref()
            .and_then(|session| session.client.refresh_token().map(str::to_owned))
    }

    fn snapshot(&self) -> Result<Snapshot, Error> {
        on_session(|session| {
            session.runtime.block_on(async {
                let torrents = session.client.torrents_with_stats().await?;
                let stats = session.client.stats().await?;
                Ok(Snapshot { torrents, stats })
            })
        })
    }

    fn act(&self, id: u64, action: RowAction) -> Result<(), Error> {
        on_session(|session| {
            session.runtime.block_on(async {
                match action {
                    RowAction::Start => session.client.start_torrent(id).await,
                    RowAction::Pause => session.client.pause_torrent(id).await,
                    RowAction::Delete => session.client.delete_torrent(id).await,
                }
            })
        })
    }

    fn add(&self, request: &AddRequest) -> Result<(), Error> {
        let source = request.source.clone();
        let save_path = request.save_path.clone();
        let category = request.category.clone();
        let start_paused = request.start_paused;
        on_session(move |session| {
            session.runtime.block_on(async {
                let path = Some(save_path.as_str());
                let category = if category.is_empty() {
                    None
                } else {
                    Some(category.as_str())
                };
                match source {
                    AddSource::Magnet(magnet) => {
                        session
                            .client
                            .add_magnet(&magnet, path, category, start_paused)
                            .await
                    }
                    AddSource::File { bytes, .. } => {
                        session
                            .client
                            .add_torrent_file(&bytes, path, category, start_paused)
                            .await
                    }
                }
            })
        })
    }
    fn detail(&self, id: u64) -> Result<DetailSnapshot, Error> {
        on_session(|session| {
            session.runtime.block_on(async {
                let torrent = session.client.torrent(id).await?;
                let trackers = session.client.trackers(id).await?;
                let peers = session.client.peers(id).await?;
                let pieces = session.client.haves(id).await?;
                Ok(DetailSnapshot {
                    torrent,
                    trackers,
                    peers,
                    pieces,
                })
            })
        })
    }

    fn detail_act(&self, id: u64, action: &DetailAction) -> Result<(), Error> {
        let action = action.clone();
        on_session(move |session| {
            session.runtime.block_on(async {
                match action {
                    DetailAction::SetIncluded { index, included } => {
                        post_inclusion(&mut session.client, id, &[index], included).await
                    }
                    DetailAction::SetIncludedMany { indices, included } => {
                        post_inclusion(&mut session.client, id, &indices, included).await
                    }
                    DetailAction::AddTracker(url) => session.client.add_trackers(id, &[url]).await,
                }
            })
        })
    }

    fn load_settings(&self) -> Result<SettingsSnapshot, Error> {
        on_session(|session| session.runtime.block_on(session.client.settings()))
    }

    fn save_settings(&self, changes: &SettingsChanges) -> Result<(), Error> {
        let changes = changes.clone();
        on_session(move |session| {
            session.runtime.block_on(async {
                if let Some(body) = &changes.network {
                    session.client.set_network(body).await?;
                }
                if let Some(body) = &changes.downloads {
                    session.client.set_downloads(body).await?;
                }
                if let Some(body) = &changes.advanced {
                    session.client.set_advanced(body).await?;
                }
                if let Some(body) = &changes.proxy {
                    session.client.set_proxy(body).await?;
                }
                if let Some(body) = &changes.ip_filter {
                    session.client.set_ip_filter(body).await?;
                }
                if let Some(body) = &changes.folders {
                    session.client.set_folders(body).await?;
                }
                if let Some(body) = &changes.limits {
                    session.client.set_limits(body).await?;
                }
                if let Some(enabled) = changes.alt_enabled {
                    session.client.set_alt_enabled(enabled).await?;
                }
                if let Some(body) = &changes.alt_config {
                    session.client.set_alt_config(body).await?;
                }
                if let Some(body) = &changes.schedule {
                    session.client.set_schedule(body).await?;
                }
                if let Some(body) = &changes.seed_limits {
                    session.client.set_seed_limits(body).await?;
                }
                Ok(())
            })
        })
    }

    fn load_rss(&self) -> Result<RssSnapshot, Error> {
        on_session(|session| session.runtime.block_on(session.client.rss()))
    }

    fn download_rss(&self, id: &str) -> Result<(), Error> {
        on_session(|session| {
            session
                .runtime
                .block_on(session.client.download_rss_item(id))
        })
    }

    fn save_rss_rule(&self, id: Option<&str>, body: &RssRuleBody) -> Result<(), Error> {
        on_session(|session| {
            session.runtime.block_on(async {
                match id {
                    Some(id) => session.client.update_rss_rule(id, body).await,
                    None => session.client.add_rss_rule(body).await,
                }
            })
        })
    }

    fn delete_rss_rule(&self, id: &str) -> Result<(), Error> {
        on_session(|session| session.runtime.block_on(session.client.delete_rss_rule(id)))
    }

    fn rss_rule_episodes(&self, id: &str) -> Result<Vec<RssRuleEpisode>, Error> {
        on_session(|session| {
            session
                .runtime
                .block_on(session.client.rss_rule_episodes(id))
        })
    }
}

fn on_session<T>(call: impl FnOnce(&mut Session) -> Result<T, Error>) -> Result<T, Error> {
    let mut guard = sessions().lock().unwrap_or_else(|err| err.into_inner());
    let session = guard.as_mut().ok_or(Error::Unauthorised)?;
    call(session)
}

fn runtime() -> Result<tokio::runtime::Runtime, Error> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| Error::Unreachable(format!("could not start the client: {err}")))
}

/// The pane reports which files changed. The posted selection is every file
/// the last poll marked included, with those files set to `included`.
async fn post_inclusion(
    client: &mut Client,
    id: u64,
    changed: &[usize],
    included: bool,
) -> Result<(), Error> {
    let torrent = client.torrent(id).await?;
    let files = torrent.files.unwrap_or_default();
    let changed_set: std::collections::BTreeSet<usize> = changed.iter().copied().collect();
    let mut indices: Vec<usize> = files
        .iter()
        .enumerate()
        .filter_map(|(i, file)| file.included.then_some(i))
        .filter(|i| !changed_set.contains(i))
        .collect();
    if included {
        indices.extend(changed.iter().copied());
        indices.sort_unstable();
        indices.dedup();
    }
    client.update_only_files(id, &indices).await
}
