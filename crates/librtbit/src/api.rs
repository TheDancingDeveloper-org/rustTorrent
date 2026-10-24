use std::{
    collections::{HashMap, HashSet},
    marker::PhantomData,
    net::SocketAddr,
    num::NonZeroU16,
    path::PathBuf,
    str::FromStr,
    sync::Arc,
};

use anyhow::Context;
use buffers::ByteBufOwned;
use dht::{DhtStats, Id20};
use http::StatusCode;
use librtbit_core::torrent_metainfo::{FileDetailsAttrs, ValidatedTorrentMetaV1Info};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    WithStatus, WithStatusError,
    api_error::ApiError,
    category::TorrentCategory,
    session::{
        AddTorrent, AddTorrentOptions, AddTorrentResponse, ListOnlyResponse, Session, TorrentId,
    },
    session_persistence::TorrentRateLimits,
    session_stats::snapshot::SessionStatsSnapshot,
    tag::normalize_tags,
    torrent_state::{
        FileStream, ManagedTorrentHandle,
        peer::stats::snapshot::{PeerStatsFilter, PeerStatsSnapshot},
    },
    type_aliases::BF,
};

#[cfg(feature = "tracing-subscriber-utils")]
use crate::tracing_subscriber_config_utils::LineBroadcast;
#[cfg(feature = "tracing-subscriber-utils")]
use futures::{Stream, StreamExt, stream};
#[cfg(feature = "tracing-subscriber-utils")]
use tokio_stream::wrappers::{BroadcastStream, errors::BroadcastStreamRecvError};

pub use crate::torrent_state::stats::{LiveStats, TorrentStats};

pub type Result<T> = std::result::Result<T, ApiError>;

/// Library API for use in different web frameworks.
/// Contains all methods you might want to expose with (de)serializable inputs/outputs.
#[derive(Clone)]
pub struct Api {
    session: Arc<Session>,
    rust_log_reload_tx: Option<UnboundedSender<String>>,
    #[cfg(feature = "tracing-subscriber-utils")]
    line_broadcast: Option<LineBroadcast>,
}

#[derive(Debug, Clone, Copy)]
pub enum TorrentIdOrHash {
    Id(TorrentId),
    Hash(Id20),
}

impl Serialize for TorrentIdOrHash {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            TorrentIdOrHash::Id(id) => id.serialize(serializer),
            TorrentIdOrHash::Hash(h) => h.as_string().serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for TorrentIdOrHash {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Default)]
        struct V<'de> {
            p: PhantomData<&'de ()>,
        }

        macro_rules! visit_int {
            ($v:expr) => {{
                let tid: TorrentId = $v.try_into().map_err(|e| E::custom(format!("{e:#}")))?;
                Ok(TorrentIdOrHash::from(tid))
            }};
        }

        impl<'de> serde::de::Visitor<'de> for V<'de> {
            type Value = TorrentIdOrHash;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("integer or 40 byte info hash")
            }

            fn visit_i64<E>(self, v: i64) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                visit_int!(v)
            }

            fn visit_i128<E>(self, v: i128) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                visit_int!(v)
            }

            fn visit_u128<E>(self, v: u128) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                visit_int!(v)
            }

            fn visit_u64<E>(self, v: u64) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                visit_int!(v)
            }

            fn visit_str<E>(self, v: &str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                TorrentIdOrHash::parse(v).map_err(|e| {
                    E::custom(format!(
                        "expected integer or 40 byte info hash, couldn't parse string: {e:#}"
                    ))
                })
            }
        }

        deserializer.deserialize_any(V::default())
    }
}

impl std::fmt::Display for TorrentIdOrHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TorrentIdOrHash::Id(id) => write!(f, "{id}"),
            TorrentIdOrHash::Hash(h) => write!(f, "{h:?}"),
        }
    }
}

impl From<TorrentId> for TorrentIdOrHash {
    fn from(value: TorrentId) -> Self {
        TorrentIdOrHash::Id(value)
    }
}

impl From<Id20> for TorrentIdOrHash {
    fn from(value: Id20) -> Self {
        TorrentIdOrHash::Hash(value)
    }
}

impl<'a> TryFrom<&'a str> for TorrentIdOrHash {
    type Error = anyhow::Error;

    fn try_from(value: &'a str) -> std::result::Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl TorrentIdOrHash {
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        if s.len() == 40 {
            let id = Id20::from_str(s)?;
            return Ok(id.into());
        }
        let id: TorrentId = s.parse()?;
        Ok(id.into())
    }
}

#[derive(Deserialize, Default)]
#[cfg_attr(feature = "swagger", derive(utoipa::IntoParams))]
pub struct ApiTorrentListOpts {
    #[serde(default)]
    pub with_stats: bool,
    /// Offset for pagination (skip this many torrents). Default: 0.
    pub offset: Option<usize>,
    /// Maximum number of torrents to return. Default: all.
    pub limit: Option<usize>,
}

impl Api {
    pub fn new(
        session: Arc<Session>,
        rust_log_reload_tx: Option<UnboundedSender<String>>,
        #[cfg(feature = "tracing-subscriber-utils")] line_broadcast: Option<LineBroadcast>,
    ) -> Self {
        Self {
            session,
            rust_log_reload_tx,
            #[cfg(feature = "tracing-subscriber-utils")]
            line_broadcast,
        }
    }

    pub fn session(&self) -> &Arc<Session> {
        &self.session
    }

    pub fn mgr_handle(&self, idx: TorrentIdOrHash) -> Result<ManagedTorrentHandle> {
        self.session
            .get(idx)
            .ok_or(ApiError::torrent_not_found(idx))
    }

    pub fn api_torrent_list(&self) -> TorrentListResponse {
        self.api_torrent_list_ext(ApiTorrentListOpts {
            with_stats: false,
            ..Default::default()
        })
    }

    pub fn api_torrent_list_ext(&self, opts: ApiTorrentListOpts) -> TorrentListResponse {
        let offset = opts.offset.unwrap_or(0);
        let limit = opts.limit;
        self.session.with_torrents(|torrents| {
            let all: Vec<_> = torrents.collect();
            let total = all.len();
            let iter = all.into_iter().skip(offset);
            let iter: Box<dyn Iterator<Item = _>> = match limit {
                Some(limit) => Box::new(iter.take(limit)),
                None => Box::new(iter),
            };
            let items = iter
                .map(|(id, mgr)| {
                    let total_pieces = mgr
                        .metadata
                        .load()
                        .as_ref()
                        .map(|m| m.info.lengths().total_pieces())
                        .unwrap_or(0);
                    let mut r = TorrentDetailsResponse {
                        id: Some(id),
                        info_hash: mgr.shared().info_hash.as_string(),
                        name: mgr.name(),
                        output_folder: mgr.output_folder().to_string_lossy().into_owned(),
                        total_pieces,

                        // These will be filled in /details and /stats endpoints
                        files: None,
                        stats: None,
                        category: mgr.shared().category.read().clone(),
                        added_on: Some(mgr.shared().added_on),
                        trackers: Some(
                            mgr.shared()
                                .trackers
                                .read()
                                .iter()
                                .map(|t| t.to_string())
                                .collect(),
                        ),
                    };
                    if opts.with_stats {
                        r.stats = Some(mgr.stats());
                    }
                    r
                })
                .collect();
            TorrentListResponse {
                torrents: items,
                total,
            }
        })
    }

    pub fn api_torrent_details(&self, idx: TorrentIdOrHash) -> Result<TorrentDetailsResponse> {
        let handle = self.mgr_handle(idx)?;
        let info_hash = handle.shared().info_hash;
        let only_files = handle.only_files();
        let category = handle.shared().category.read().clone();
        let output_folder = handle.output_folder().to_string_lossy().into_owned();
        make_torrent_details(
            Some(handle.id()),
            &info_hash,
            handle.metadata.load().as_ref().map(|r| &r.info),
            handle.name().as_deref(),
            only_files.as_deref(),
            output_folder,
            category,
        )
    }

    pub fn api_session_stats(&self) -> SessionStatsSnapshot {
        self.session().stats_snapshot()
    }

    pub fn torrent_file_mime_type(
        &self,
        idx: TorrentIdOrHash,
        file_idx: usize,
    ) -> Result<&'static str> {
        let handle = self.mgr_handle(idx)?;
        handle.with_metadata(|r| torrent_file_mime_type(&r.info, file_idx))?
    }

    pub fn api_peer_stats(
        &self,
        idx: TorrentIdOrHash,
        filter: PeerStatsFilter,
    ) -> Result<PeerStatsSnapshot> {
        let handle = self.mgr_handle(idx)?;
        Ok(handle
            .live()
            .with_status_error(
                StatusCode::PRECONDITION_FAILED,
                crate::Error::TorrentIsNotLive,
            )?
            .per_peer_stats_snapshot(filter))
    }

    pub async fn api_torrent_action_pause(
        &self,
        idx: TorrentIdOrHash,
    ) -> Result<EmptyJsonResponse> {
        let handle = self.mgr_handle(idx)?;
        self.session()
            .pause(&handle)
            .await
            .with_status(StatusCode::BAD_REQUEST)?;
        Ok(Default::default())
    }

    pub async fn api_torrent_action_start(
        &self,
        idx: TorrentIdOrHash,
    ) -> Result<EmptyJsonResponse> {
        let handle = self.mgr_handle(idx)?;
        self.session
            .unpause(&handle)
            .await
            .with_status(StatusCode::BAD_REQUEST)?;
        Ok(Default::default())
    }

    pub async fn api_torrent_action_recheck(
        &self,
        idx: TorrentIdOrHash,
    ) -> Result<EmptyJsonResponse> {
        let handle = self.mgr_handle(idx)?;
        self.session
            .force_recheck(&handle)
            .await
            .with_status(StatusCode::BAD_REQUEST)?;
        Ok(Default::default())
    }

    pub async fn api_torrent_action_add_trackers(
        &self,
        idx: TorrentIdOrHash,
        trackers: Vec<String>,
    ) -> Result<EmptyJsonResponse> {
        let handle = self.mgr_handle(idx)?;
        self.session
            .add_trackers(&handle, trackers)
            .await
            .with_status(StatusCode::BAD_REQUEST)?;
        Ok(Default::default())
    }

    pub async fn api_torrent_action_remove_trackers(
        &self,
        idx: TorrentIdOrHash,
        trackers: Vec<String>,
    ) -> Result<RemoveTrackersResponse> {
        let handle = self.mgr_handle(idx)?;
        let removed = self
            .session
            .remove_trackers(&handle, &trackers)
            .await
            .with_status(StatusCode::BAD_REQUEST)?;
        Ok(RemoveTrackersResponse { removed })
    }

    pub async fn api_torrent_action_edit_tracker(
        &self,
        idx: TorrentIdOrHash,
        orig_url: String,
        new_url: String,
    ) -> Result<EmptyJsonResponse> {
        let handle = self.mgr_handle(idx)?;
        self.session
            .edit_tracker(&handle, &orig_url, &new_url)
            .await
            .with_status(StatusCode::BAD_REQUEST)?;
        Ok(Default::default())
    }

    /// Ask every tracker to be announced to now. Returns whether the torrent
    /// was live (a paused torrent has no announce loops to wake).
    pub fn api_torrent_action_reannounce(
        &self,
        idx: TorrentIdOrHash,
    ) -> Result<ReannounceResponse> {
        let handle = self.mgr_handle(idx)?;
        Ok(ReannounceResponse {
            live: handle.reannounce(),
        })
    }

    pub async fn api_torrent_action_forget(
        &self,
        idx: TorrentIdOrHash,
    ) -> Result<EmptyJsonResponse> {
        self.session
            .delete(idx, false)
            .await
            .context("error forgetting torrent")?;
        Ok(Default::default())
    }

    pub async fn api_torrent_action_delete(
        &self,
        idx: TorrentIdOrHash,
    ) -> Result<EmptyJsonResponse> {
        self.session
            .delete(idx, true)
            .await
            .context("error deleting torrent with files")?;
        Ok(Default::default())
    }

    pub async fn api_torrent_action_update_only_files(
        &self,
        idx: TorrentIdOrHash,
        only_files: &HashSet<usize>,
    ) -> Result<EmptyJsonResponse> {
        let handle = self.mgr_handle(idx)?;
        self.session
            .update_only_files(&handle, only_files)
            .await
            .context("error updating only_files")?;
        Ok(Default::default())
    }

    pub fn api_set_rust_log(&self, new_value: String) -> Result<EmptyJsonResponse> {
        let tx = self
            .rust_log_reload_tx
            .as_ref()
            .context("rust_log_reload_tx was not set")?;
        tx.send(new_value)
            .context("noone is listening to RUST_LOG changes")?;
        Ok(Default::default())
    }

    #[cfg(feature = "tracing-subscriber-utils")]
    pub fn api_log_lines_stream(
        &self,
    ) -> Result<
        impl Stream<Item = std::result::Result<bytes::Bytes, BroadcastStreamRecvError>>
        + Send
        + Sync
        + 'static,
    > {
        let (history, receiver) = self
            .line_broadcast
            .as_ref()
            .map(LineBroadcast::subscribe)
            .context("line_rx wasn't set")?;
        Ok(stream::iter(history.into_iter().map(Ok)).chain(BroadcastStream::new(receiver)))
    }

    pub async fn api_add_torrent(
        &self,
        add: AddTorrent<'_>,
        opts: Option<AddTorrentOptions>,
    ) -> Result<ApiAddTorrentResponse> {
        let response = match self
            .session
            .add_torrent(add, opts)
            .await
            .context("error adding torrent")
            .with_status(StatusCode::BAD_REQUEST)?
        {
            AddTorrentResponse::AlreadyManaged(id, handle) => {
                let details = make_torrent_details(
                    Some(id),
                    &handle.info_hash(),
                    handle.metadata.load().as_ref().map(|r| &r.info),
                    handle.name().as_deref(),
                    handle.only_files().as_deref(),
                    handle
                        .shared()
                        .options
                        .output_folder
                        .to_string_lossy()
                        .into_owned(),
                    handle.shared().category.read().clone(),
                )
                .context("error making torrent details")?;
                ApiAddTorrentResponse {
                    id: Some(id),
                    details,
                    seen_peers: None,
                    output_folder: handle
                        .shared()
                        .options
                        .output_folder
                        .to_string_lossy()
                        .into_owned(),
                }
            }
            AddTorrentResponse::ListOnly(ListOnlyResponse {
                info_hash,
                info,
                only_files,
                seen_peers,
                output_folder,
                ..
            }) => ApiAddTorrentResponse {
                id: None,
                output_folder: output_folder.to_string_lossy().into_owned(),
                seen_peers: Some(seen_peers),
                details: make_torrent_details(
                    None,
                    &info_hash,
                    Some(&info),
                    None,
                    only_files.as_deref(),
                    output_folder.to_string_lossy().into_owned().to_string(),
                    None,
                )
                .context("error making torrent details")?,
            },
            AddTorrentResponse::Added(id, handle) => {
                let details = make_torrent_details(
                    Some(id),
                    &handle.info_hash(),
                    handle.metadata.load().as_ref().map(|r| &r.info),
                    handle.name().as_deref(),
                    handle.only_files().as_deref(),
                    handle
                        .shared()
                        .options
                        .output_folder
                        .to_string_lossy()
                        .into_owned(),
                    handle.shared().category.read().clone(),
                )
                .context("error making torrent details")?;
                ApiAddTorrentResponse {
                    id: Some(id),
                    details,
                    seen_peers: None,
                    output_folder: handle
                        .shared()
                        .options
                        .output_folder
                        .to_string_lossy()
                        .into_owned(),
                }
            }
        };
        Ok(response)
    }

    pub fn api_completed_folder(&self) -> Option<String> {
        self.session
            .completed_folder()
            .map(|p| p.to_string_lossy().into_owned())
    }

    pub fn api_output_folder(&self) -> String {
        self.session.output_folder().to_string_lossy().into_owned()
    }

    pub fn api_listen_port(&self) -> u16 {
        self.session.listen_addr().map_or(0, |addr| addr.port())
    }

    pub fn api_announce_port(&self) -> u16 {
        self.session.announce_port().unwrap_or_default()
    }

    pub fn api_set_announce_port(&self, port: NonZeroU16) {
        self.session.set_announce_port(port);
    }

    pub fn api_set_output_folder(&self, folder: String) {
        self.session.set_output_folder(PathBuf::from(folder));
    }

    pub fn api_set_completed_folder(&self, folder: Option<String>) {
        self.session.set_completed_folder(folder.map(PathBuf::from));
    }

    pub async fn api_set_folders(
        &self,
        download_folder: String,
        completed_folder: Option<String>,
    ) -> Result<()> {
        self.api_set_output_folder(download_folder);
        self.api_set_completed_folder(completed_folder);
        self.session
            .persist_session_settings()
            .await
            .map_err(ApiError::from)?;
        Ok(())
    }

    pub fn api_dht_stats(&self) -> Result<DhtStats> {
        self.session
            .get_dht()
            .as_ref()
            .map(|d| d.stats())
            .ok_or(ApiError::dht_disabled())
    }

    pub fn api_dht_table(&self) -> Result<impl Serialize + use<>> {
        let dht = self.session.get_dht().ok_or(ApiError::dht_disabled())?;
        Ok(dht.with_routing_tables(|v4, v6| {
            #[derive(Serialize)]
            struct Tables<T> {
                v4: T,
                v6: T,
            }
            Tables {
                v4: v4.clone(),
                v6: v6.clone(),
            }
        }))
    }

    pub fn api_stats_v0(&self, idx: TorrentIdOrHash) -> Result<LiveStats> {
        let mgr = self.mgr_handle(idx)?;
        let live = mgr.live().context("torrent not live")?;
        Ok(LiveStats::from(&*live))
    }

    pub fn api_stats_v1(&self, idx: TorrentIdOrHash) -> Result<TorrentStats> {
        let mgr = self.mgr_handle(idx)?;
        Ok(mgr.stats())
    }

    pub fn api_tracker_status(&self, idx: TorrentIdOrHash) -> Result<TrackerStatusResponse> {
        let mgr = self.mgr_handle(idx)?;
        let registry = &mgr.shared().tracker_status;
        // Make sure every configured tracker shows up, even before any announce.
        for t in mgr.shared().trackers.read().iter() {
            registry.ensure(t.as_str());
        }
        Ok(TrackerStatusResponse {
            trackers: registry.snapshot(),
        })
    }

    pub fn api_dump_haves(&self, idx: TorrentIdOrHash) -> Result<(BF, u32)> {
        let mgr = self.mgr_handle(idx)?;
        Ok(mgr.with_chunk_tracker(|chunks| {
            let bf = BF::from_bitslice(chunks.get_have_pieces().as_slice());
            let len = chunks.get_lengths().total_pieces();
            (bf, len)
        })?)
    }

    pub async fn api_stream(&self, idx: TorrentIdOrHash, file_id: usize) -> Result<FileStream> {
        let mgr = self.mgr_handle(idx)?;
        Ok(mgr.stream(file_id).await?)
    }

    pub fn api_list_categories(&self) -> HashMap<String, TorrentCategory> {
        self.session.categories().list()
    }

    pub async fn api_create_or_edit_category(
        &self,
        name: String,
        save_path: Option<PathBuf>,
    ) -> Result<EmptyJsonResponse> {
        let mgr = self.session.categories();
        if mgr.exists(&name) {
            mgr.edit(&name, save_path)
                .context("error editing category")?;
        } else {
            mgr.create(name, save_path)
                .context("error creating category")?;
        }
        self.session
            .persist_categories()
            .await
            .context("error persisting categories")?;
        Ok(Default::default())
    }

    pub async fn api_remove_category(&self, name: &str) -> Result<EmptyJsonResponse> {
        self.session
            .categories()
            .remove(name)
            .context("error removing category")?;
        self.session
            .persist_categories()
            .await
            .context("error persisting categories")?;
        Ok(Default::default())
    }

    pub fn api_torrent_rate_limits(&self, idx: TorrentIdOrHash) -> Result<TorrentRateLimits> {
        Ok(TorrentRateLimits::of(&self.mgr_handle(idx)?))
    }

    /// Set a torrent's own speed limits, layered under the session limits.
    /// Applied to the live limiter at once and saved with the torrent.
    pub async fn api_set_torrent_rate_limits(
        &self,
        idx: TorrentIdOrHash,
        limits: TorrentRateLimits,
    ) -> Result<EmptyJsonResponse> {
        let handle = self.mgr_handle(idx)?;
        handle.set_download_limit(limits.download_bps);
        handle.set_upload_limit(limits.upload_bps);
        self.session.try_update_persistence_metadata(&handle).await;
        Ok(Default::default())
    }

    pub async fn api_set_torrent_category(
        &self,
        idx: TorrentIdOrHash,
        category: Option<String>,
    ) -> Result<EmptyJsonResponse> {
        self.session
            .set_torrent_category(idx, category)
            .context("error setting torrent category")?;
        // Persist the torrent metadata to save the category
        let handle = self.mgr_handle(idx)?;
        self.session.try_update_persistence_metadata(&handle).await;
        Ok(Default::default())
    }

    pub fn api_list_tags(&self) -> Vec<String> {
        self.session.tags().list()
    }

    /// Register tags without assigning them to any torrent.
    pub async fn api_create_tags(&self, tags: &[String]) -> Result<EmptyJsonResponse> {
        if self.session.tags().create(&normalize_tags(tags)) {
            self.persist_tags().await?;
        }
        Ok(Default::default())
    }

    /// Unregister tags and strip them from every torrent carrying them.
    pub async fn api_delete_tags(&self, tags: &[String]) -> Result<EmptyJsonResponse> {
        let tags = normalize_tags(tags);
        self.session.tags().delete(&tags);
        let affected: Vec<ManagedTorrentHandle> = self.session.with_torrents(|iter| {
            iter.filter(|(_, handle)| {
                let mut own = handle.shared().tags.write();
                let before = own.len();
                own.retain(|tag| !tags.contains(tag));
                own.len() != before
            })
            .map(|(_, handle)| handle.clone())
            .collect()
        });
        for handle in affected {
            self.session.try_update_persistence_metadata(&handle).await;
        }
        self.persist_tags().await?;
        Ok(Default::default())
    }

    /// Add tags to a torrent, registering any that are new.
    pub async fn api_add_torrent_tags(
        &self,
        idx: TorrentIdOrHash,
        tags: &[String],
    ) -> Result<EmptyJsonResponse> {
        let tags = normalize_tags(tags);
        self.update_torrent_tags(idx, |own| own.extend(tags)).await
    }

    /// Remove tags from a torrent. An empty list removes every tag, matching
    /// the WebUI API v2 semantics.
    pub async fn api_remove_torrent_tags(
        &self,
        idx: TorrentIdOrHash,
        tags: &[String],
    ) -> Result<EmptyJsonResponse> {
        let tags = normalize_tags(tags);
        self.update_torrent_tags(idx, |own| {
            if tags.is_empty() {
                own.clear();
            } else {
                own.retain(|tag| !tags.contains(tag));
            }
        })
        .await
    }

    /// Replace a torrent's tag set, registering any tags that are new.
    pub async fn api_set_torrent_tags(
        &self,
        idx: TorrentIdOrHash,
        tags: &[String],
    ) -> Result<EmptyJsonResponse> {
        let tags = normalize_tags(tags);
        self.update_torrent_tags(idx, |own| *own = tags).await
    }

    async fn update_torrent_tags(
        &self,
        idx: TorrentIdOrHash,
        f: impl FnOnce(&mut std::collections::BTreeSet<String>),
    ) -> Result<EmptyJsonResponse> {
        let registered_before = self.session.tags().snapshot().len();
        let changed = self
            .session
            .update_torrent_tags(idx, f)
            .context("error setting torrent tags")?;
        if changed {
            let handle = self.mgr_handle(idx)?;
            self.session.try_update_persistence_metadata(&handle).await;
        }
        if self.session.tags().snapshot().len() != registered_before {
            self.persist_tags().await?;
        }
        Ok(Default::default())
    }

    async fn persist_tags(&self) -> Result<()> {
        self.session
            .persist_tags()
            .await
            .context("error persisting tags")?;
        Ok(())
    }
}

#[derive(Serialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
pub struct TorrentListResponse {
    pub torrents: Vec<TorrentDetailsResponse>,
    /// Total number of torrents (before pagination).
    pub total: usize,
}

#[derive(Serialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
pub struct TrackerStatusResponse {
    #[cfg_attr(feature = "swagger", schema(value_type = Vec<Object>))]
    pub trackers: Vec<tracker_comms::TrackerStatus>,
}

#[derive(Serialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
pub struct RemoveTrackersResponse {
    /// How many of the requested URLs were configured and removed.
    pub removed: usize,
}

#[derive(Serialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
pub struct ReannounceResponse {
    /// False when the torrent is paused and there was nothing to announce.
    pub live: bool,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
pub struct TorrentDetailsResponseFile {
    pub name: String,
    pub components: Vec<String>,
    pub length: u64,
    pub included: bool,
    #[cfg_attr(feature = "swagger", schema(value_type = Object))]
    pub attributes: FileDetailsAttrs,
}

#[derive(Default, Serialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
pub struct EmptyJsonResponse {}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
pub struct TorrentDetailsResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<usize>,
    pub info_hash: String,
    pub name: Option<String>,
    pub output_folder: String,

    #[serde(default)]
    pub total_pieces: u32,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<TorrentDetailsResponseFile>>,
    #[serde(skip_serializing_if = "Option::is_none", skip_deserializing)]
    #[cfg_attr(feature = "swagger", schema(value_type = Object))]
    pub stats: Option<TorrentStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Unix timestamp of when the torrent was added to the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_on: Option<u64>,
    /// Announce URLs configured for this torrent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trackers: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
pub struct ApiAddTorrentResponse {
    pub id: Option<usize>,
    pub details: TorrentDetailsResponse,
    pub output_folder: String,
    #[cfg_attr(feature = "swagger", schema(value_type = Option<Vec<String>>))]
    pub seen_peers: Option<Vec<SocketAddr>>,
}

fn make_torrent_details(
    id: Option<TorrentId>,
    info_hash: &Id20,
    info: Option<&ValidatedTorrentMetaV1Info<ByteBufOwned>>,
    name: Option<&str>,
    only_files: Option<&[usize]>,
    output_folder: String,
    category: Option<String>,
) -> Result<TorrentDetailsResponse> {
    let files = match info {
        Some(info) => info
            .iter_file_details()
            .enumerate()
            .map(|(idx, d)| {
                let name = d.filename.to_string();
                let components = d.filename.to_vec();
                let included = only_files.map(|o| o.contains(&idx)).unwrap_or(true);
                TorrentDetailsResponseFile {
                    name,
                    components,
                    length: d.len,
                    included,
                    attributes: d.attrs(),
                }
            })
            .collect(),
        None => Default::default(),
    };
    let total_pieces = info.map(|i| i.lengths().total_pieces()).unwrap_or(0);
    Ok(TorrentDetailsResponse {
        id,
        info_hash: info_hash.as_string(),
        name: name
            .map(|s| s.to_owned())
            .or_else(|| info.and_then(|i| i.name().map(|n| n.into_owned()))),
        files: Some(files),
        output_folder,
        total_pieces,
        stats: None,
        category,
        added_on: None,
        trackers: None,
    })
}

fn torrent_file_mime_type(
    info: &ValidatedTorrentMetaV1Info<ByteBufOwned>,
    file_idx: usize,
) -> Result<&'static str> {
    Ok(info
        .iter_file_details()
        .nth(file_idx)
        .and_then(|d| {
            d.filename
                .iter_components()
                .last()
                .and_then(|s| mime_guess::from_path(&*s).first_raw())
        })
        .ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "cannot determine mime type for file",
        ))?)
}
