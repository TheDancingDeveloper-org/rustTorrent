use std::{
    borrow::Cow,
    collections::HashMap,
    io::Read,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::Context;
use buffers::{ByteBuf, ByteBufOwned};
use bytes::Bytes;
use clone_to_owned::CloneToOwned;
use dht::Id20;
use librtbit_core::{
    directories::get_configuration_directory,
    magnet::Magnet,
    torrent_metainfo::{TorrentMetaV1Owned, ValidatedTorrentMetaV1Info},
};
use serde::{Deserialize, Serialize};
use tracing::trace;

use crate::{
    limits::LimitsConfig,
    listen::ListenerOptions,
    peer_connection::PeerConnectionOptions,
    read_buf::ReadBuf,
    storage::BoxStorageFactory,
    stream_connect::{ConnectionKind, ConnectionOptions},
    torrent_state::{ManagedTorrentHandle, TorrentMetadata},
    type_aliases::{BoxAsyncReadVectored, BoxAsyncWrite, PeerStream},
};
use peer_binary_protocol::Handshake;

use super::{Session, TorrentId};

pub(crate) struct ParsedTorrentFile {
    pub meta: TorrentMetaV1Owned,
    pub torrent_bytes: Bytes,
}

pub(crate) fn torrent_from_bytes(bytes: Bytes) -> anyhow::Result<ParsedTorrentFile> {
    trace!(
        "all fields in torrent: {:#?}",
        bencode::dyn_from_bytes::<ByteBuf>(&bytes)
    );
    let parsed = librtbit_core::torrent_metainfo::torrent_from_bytes(&bytes)?;
    Ok(ParsedTorrentFile {
        meta: parsed.clone_to_owned(Some(&bytes)),
        torrent_bytes: bytes,
    })
}

#[derive(Default)]
pub struct SessionDatabase {
    pub(crate) torrents: HashMap<TorrentId, ManagedTorrentHandle>,
    /// Reverse index for O(1) lookup by info_hash (used in incoming peer handshakes).
    pub(crate) info_hash_to_id: HashMap<Id20, TorrentId>,
}

impl SessionDatabase {
    pub(crate) fn add_torrent(&mut self, torrent: ManagedTorrentHandle, id: TorrentId) {
        let info_hash = torrent.info_hash();
        self.info_hash_to_id.insert(info_hash, id);
        self.torrents.insert(id, torrent);
    }

    pub(crate) fn remove_torrent(&mut self, id: &TorrentId) -> Option<ManagedTorrentHandle> {
        let removed = self.torrents.remove(id);
        if let Some(ref handle) = removed {
            self.info_hash_to_id.remove(&handle.info_hash());
        }
        removed
    }

    /// O(1) lookup by info_hash.
    pub(crate) fn get_by_info_hash(
        &self,
        info_hash: Id20,
    ) -> Option<(TorrentId, &ManagedTorrentHandle)> {
        let id = self.info_hash_to_id.get(&info_hash)?;
        let handle = self.torrents.get(id)?;
        Some((*id, handle))
    }
}

/// Options for adding new torrents to the session.
//
// Serialize/deserialize is for Tauri.
#[derive(Default, Serialize, Deserialize)]
pub struct AddTorrentOptions {
    /// Start in paused state.
    #[serde(default)]
    pub paused: bool,
    /// With `paused`, don't hash-check the payload until the torrent is
    /// started or rechecked: the torrent reports as stopped with nothing
    /// verified. Bulk imports onto existing (possibly read-only) payloads use
    /// this so adding hundreds of torrents doesn't start hundreds of checks.
    /// A restart re-adds the torrent paused and checks it as usual.
    #[serde(default)]
    pub defer_initial_check: bool,
    /// Store incomplete files under `.part` names. `None` follows the session
    /// setting (see `Session::set_use_part_files`). Restored torrents set this
    /// to the choice they were added with.
    #[serde(skip)]
    pub part_files: Option<bool>,
    /// Files to store under a path other than the one in the info dict, as
    /// `(file_id, relative_path)`. Restored torrents set this to the renames
    /// made while they were managed.
    #[serde(skip)]
    pub file_renames: Vec<(usize, PathBuf)>,
    /// A regex to only download files matching it.
    pub only_files_regex: Option<String>,
    /// An explicit list of file IDs to download.
    /// To see the file indices, run with "list_only".
    pub only_files: Option<Vec<usize>>,
    /// Allow writing on top of existing files, including when resuming a torrent.
    /// You probably want to set it, however for safety it's not default.
    ///
    /// Even when all the torrent pieces have been written, `overwrite` needs to
    /// be enabled in order to resume/seed the torrent.
    #[serde(default)]
    pub overwrite: bool,
    /// Only list the files in the torrent without starting it.
    #[serde(default)]
    pub list_only: bool,
    /// The output folder for the torrent. If not set, the session's default one will be used.
    pub output_folder: Option<String>,
    /// A root output folder under which multi-file torrents receive their
    /// normal torrent-name subfolder. This matches qBittorrent's `savepath`
    /// semantics and is mutually exclusive with `output_folder` and
    /// `sub_folder`.
    pub output_folder_root: Option<String>,
    /// Sub-folder within session's default output folder. Will error if "output_folder" if also set.
    /// By default, multi-torrent files are downloaded to a sub-folder.
    pub sub_folder: Option<String>,
    /// Peer connection options, timeouts etc. If not set, session's defaults will be used.
    pub peer_opts: Option<PeerConnectionOptions>,

    /// Force a refresh interval for polling trackers.
    pub force_tracker_interval: Option<Duration>,

    #[serde(default)]
    pub disable_trackers: bool,

    #[serde(default)]
    pub ratelimits: LimitsConfig,

    /// Initial peers to start of with.
    pub initial_peers: Option<Vec<SocketAddr>>,

    /// Max concurrent connected peers.
    pub peer_limit: Option<usize>,

    /// This is used to restore the session from serialized state.
    pub preferred_id: Option<usize>,

    #[serde(skip)]
    pub storage_factory: Option<BoxStorageFactory>,

    // Custom trackers
    pub trackers: Option<Vec<String>>,

    /// Category to assign to this torrent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,

    /// Tags to assign to this torrent. They are registered in the session's
    /// tag registry when the torrent is added.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// Original Unix timestamp when restoring a persisted torrent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_on: Option<u64>,

    /// Completion Unix timestamp when restoring a persisted torrent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_on: Option<u64>,

    /// Timeout for magnet link metadata resolution, in seconds.
    /// If not set, defaults to 120 seconds. Set to 0 to disable the timeout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magnet_resolution_timeout_secs: Option<u64>,
}

pub struct ListOnlyResponse {
    pub info_hash: Id20,
    pub info: ValidatedTorrentMetaV1Info<ByteBufOwned>,
    pub only_files: Option<Vec<usize>>,
    pub output_folder: PathBuf,
    pub seen_peers: Vec<SocketAddr>,
    pub torrent_bytes: Bytes,
}

#[allow(clippy::large_enum_variant)]
pub enum AddTorrentResponse {
    AlreadyManaged(TorrentId, ManagedTorrentHandle),
    ListOnly(ListOnlyResponse),
    Added(TorrentId, ManagedTorrentHandle),
}

impl AddTorrentResponse {
    pub fn into_handle(self) -> Option<ManagedTorrentHandle> {
        match self {
            Self::AlreadyManaged(_, handle) => Some(handle),
            Self::ListOnly(_) => None,
            Self::Added(_, handle) => Some(handle),
        }
    }
}

pub fn read_local_file_including_stdin(filename: &str) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    if filename == "-" {
        std::io::stdin()
            .read_to_end(&mut buf)
            .context("error reading stdin")?;
    } else {
        std::fs::File::open(filename)
            .context("error opening")?
            .read_to_end(&mut buf)
            .context("error reading")?;
    }
    Ok(buf)
}

pub enum AddTorrent<'a> {
    Url(Cow<'a, str>),
    TorrentFileBytes(Bytes),
}

impl<'a> AddTorrent<'a> {
    // Don't call this from HTTP API.
    #[inline(never)]
    pub fn from_cli_argument(path: &'a str) -> anyhow::Result<Self> {
        if super::SUPPORTED_SCHEMES.iter().any(|s| path.starts_with(s)) {
            return Ok(Self::Url(Cow::Borrowed(path)));
        }
        if path.len() == 40 && !Path::new(path).exists() && Magnet::parse(path).is_ok() {
            return Ok(Self::Url(Cow::Borrowed(path)));
        }
        Self::from_local_filename(path)
    }

    pub fn from_url(url: impl Into<Cow<'a, str>>) -> Self {
        Self::Url(url.into())
    }

    pub fn from_bytes(bytes: impl Into<Bytes>) -> Self {
        Self::TorrentFileBytes(bytes.into())
    }

    // Don't call this from HTTP API.
    #[inline(never)]
    pub fn from_local_filename(filename: &str) -> anyhow::Result<Self> {
        let file = read_local_file_including_stdin(filename)
            .with_context(|| format!("error reading local file {filename:?}"))?;
        Ok(Self::TorrentFileBytes(file.into()))
    }

    pub fn into_bytes(self) -> Bytes {
        match self {
            Self::Url(s) => s.into_owned().into_bytes().into(),
            Self::TorrentFileBytes(b) => b,
        }
    }
}

pub enum SessionPersistenceConfig {
    /// The filename for persistence. By default uses an OS-specific folder.
    Json { folder: Option<PathBuf> },
    #[cfg(feature = "postgres")]
    Postgres { connection_string: String },
}

impl SessionPersistenceConfig {
    pub fn default_json_persistence_folder() -> anyhow::Result<PathBuf> {
        let dir = get_configuration_directory("session")?;
        Ok(dir.data_dir().to_owned())
    }
}

#[derive(Default)]
pub struct SessionOptions {
    /// Turn on to disable DHT.
    pub disable_dht: bool,
    /// Turn on to disable DHT persistence. By default it will re-use stored DHT
    /// configuration, including the port it listens on.
    pub disable_dht_persistence: bool,
    /// Pass in to configure DHT persistence filename. This can be used to run multiple
    /// librtbit instances at a time.
    pub dht_config: Option<dht::PersistentDhtConfig>,
    /// A list o DHT bootstrap nodes as strings of the form host:port or ip:port
    pub dht_bootstrap_addrs: Option<Vec<String>>,

    /// What network device to bind to for DHT, BT-UDP, BT-TCP, trackers and LSD.
    /// On OSX will use IP(V6)_BOUND_IF, on Linux will use SO_BINDTODEVICE.
    pub bind_device_name: Option<String>,

    /// Disable tracker communication
    pub disable_trackers: bool,

    /// Enable fastresume, to restore state quickly after restart.
    pub fastresume: bool,

    /// Controls the intensity of probabilistic piece validation during fast resume.
    /// - `None`: uses the built-in default (denominator capped at 50).
    /// - `Some(0)`: skip probabilistic validation entirely (only per-file mandatory checks).
    /// - `Some(n)` where n > 0: cap the denominator at n. Higher = more pieces checked.
    pub fastresume_validation_denom: Option<u32>,

    /// Turn on to dump session contents into a file periodically, so that on next start
    /// all remembered torrents will continue where they left off.
    pub persistence: Option<SessionPersistenceConfig>,

    /// Do not apply the network settings saved from the web UI at this
    /// start; use `listen`, `disable_dht` and `disable_local_service_discovery`
    /// as given. The saved values are kept.
    pub ignore_saved_network: bool,

    /// How often a running session saves each torrent's state (upload totals, share
    /// limits, queue flags), so a kill loses at most this much. `None` means 60
    /// seconds. Has no effect without persistence.
    pub persist_interval: Option<Duration>,

    /// The peer ID to use. If not specified, a random one will be generated.
    pub peer_id: Option<Id20>,

    /// Options for listening on TCP and/or uTP for incoming connections.
    pub listen: Option<ListenerOptions>,
    /// Options for connecting to peers (for outgiong connections).
    pub connect: Option<ConnectionOptions>,

    pub default_storage_factory: Option<BoxStorageFactory>,

    pub cancellation_token: Option<tokio_util::sync::CancellationToken>,

    /// how many concurrent torrent initializations can happen
    pub concurrent_init_limit: Option<usize>,

    /// How many blocking threads does the tokio runtime have.
    /// Will limit blocking work to that number to avoid starving the runtime.
    pub runtime_worker_threads: Option<usize>,

    /// the root span to use. If not set will be None.
    pub root_span: Option<tracing::Span>,

    pub ratelimits: LimitsConfig,

    pub blocklist_url: Option<String>,
    pub allowlist_url: Option<String>,

    // The list of tracker URLs to always use for each torrent.
    pub trackers: std::collections::HashSet<url::Url>,

    /// Default peer limit per torrent.
    pub peer_limit: Option<usize>,

    #[cfg(feature = "disable-upload")]
    pub disable_upload: bool,

    /// Disable LSD multicast
    pub disable_local_service_discovery: bool,

    /// Force IPv4 only.
    pub ipv4_only: bool,

    /// A folder to move completed torrents to. If set, when a torrent finishes
    /// downloading, its files will be moved from the output folder to this folder.
    pub completed_folder: Option<PathBuf>,
}

pub(crate) fn torrent_file_from_info_bytes(
    info_bytes: &[u8],
    trackers: &[url::Url],
) -> anyhow::Result<Bytes> {
    #[derive(Serialize)]
    struct Tmp<'a> {
        announce: &'a str,
        #[serde(rename = "announce-list")]
        announce_list: &'a [&'a [url::Url]],
        info: bencode::raw_value::RawValue<&'a [u8]>,
    }

    let mut w = Vec::new();
    let v = Tmp {
        info: bencode::raw_value::RawValue(info_bytes),
        announce: trackers.first().map(|s| s.as_str()).unwrap_or(""),
        announce_list: &[trackers],
    };
    bencode::bencode_serialize_to_writer(&v, &mut w)?;
    Ok(w.into())
}

pub(crate) struct CheckedIncomingConnection {
    pub kind: ConnectionKind,
    pub addr: SocketAddr,
    pub reader: BoxAsyncReadVectored,
    pub writer: BoxAsyncWrite,
    pub read_buf: ReadBuf,
    pub handshake: Handshake,
}

pub(super) struct InternalAddResult {
    pub info_hash: Id20,
    pub metadata: Option<TorrentMetadata>,
    pub trackers: Vec<url::Url>,
    pub name: Option<String>,
    pub web_seed_urls: Vec<String>,
}

pub(crate) struct ResolveMagnetResult {
    pub metadata: TorrentMetadata,
    pub peer_rx: PeerStream,
    pub seen_peers: Vec<SocketAddr>,
}

// An adapter for converting stats into the format that tracker_comms accepts.
pub(super) struct PeerRxTorrentInfo {
    pub info_hash: Id20,
    pub session: Arc<Session>,
}

impl tracker_comms::TorrentStatsProvider for PeerRxTorrentInfo {
    fn get(&self) -> tracker_comms::TrackerCommsStats {
        let mt = self.session.with_torrents(|torrents| {
            for (_, mt) in torrents {
                if mt.info_hash() == self.info_hash {
                    return Some(mt.clone());
                }
            }
            None
        });
        let mt = match mt {
            Some(mt) => mt,
            None => {
                trace!(info_hash=?self.info_hash, "can't find torrent in the session, using default stats");
                return Default::default();
            }
        };
        let stats = mt.stats();

        use crate::torrent_state::stats::TorrentStatsState as TS;
        use tracker_comms::TrackerCommsStatsState as S;

        tracker_comms::TrackerCommsStats {
            downloaded_bytes: stats.progress_bytes,
            total_bytes: stats.total_bytes,
            uploaded_bytes: stats.uploaded_bytes,
            torrent_state: match stats.state {
                TS::Initializing => S::Initializing,
                TS::Live => S::Live,
                TS::Paused => S::Paused,
                TS::Error => S::None,
            },
        }
    }
}
