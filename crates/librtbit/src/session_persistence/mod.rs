pub mod json;
#[cfg(feature = "postgres")]
pub mod postgres;

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::PathBuf,
};

use anyhow::Context;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use librtbit_core::Id20;
use librtbit_core::magnet::Magnet;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    AddTorrent, AddTorrentOptions,
    bitv_factory::BitVFactory,
    category::TorrentCategory,
    session::PolicyFlags,
    session::ShareLimits,
    session::TorrentId,
    torrent_state::{ManagedTorrentHandle, ManagedTorrentShared},
};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_folder: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_folder: Option<PathBuf>,
    /// Queue limits. Absent means unlimited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_active_downloads: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_active_seeds: Option<usize>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SerializedTorrent {
    #[serde(
        serialize_with = "serialize_info_hash",
        deserialize_with = "deserialize_info_hash"
    )]
    info_hash: Id20,
    #[serde(skip)]
    torrent_bytes: Bytes,
    trackers: HashSet<String>,
    output_folder: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_folder_root: Option<PathBuf>,
    only_files: Option<Vec<usize>>,
    is_paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    category: Option<String>,
    /// Tags on the torrent. Absent in older state, which reads as no tags.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    tags: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    added_on: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completion_on: Option<u64>,
    /// Policy state from the queue, disk guard and share limits. Absent in
    /// state written before these flags were persisted, so it reads as false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    queued: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    held_for_disk: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    share_paused: bool,
    /// Queue position. Absent in older state, which falls back to the torrent id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    queue_position: Option<i64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    force_start: bool,
    /// Share limits as set on the torrent. Absent in older state, which means no limits.
    #[serde(default, skip_serializing_if = "ShareLimits::is_unset")]
    share_limits: ShareLimits,
    /// Bytes uploaded across earlier runs. Absent in older state, which reads as zero.
    #[serde(default, skip_serializing_if = "is_zero")]
    uploaded_bytes: u64,
    /// Whether incomplete files were stored under `.part` names. Absent in older
    /// state, which falls back to the session setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    part_files: Option<bool>,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

impl SerializedTorrent {
    pub fn info_hash(&self) -> &Id20 {
        &self.info_hash
    }

    /// The per-torrent state to reapply once the torrent is added back.
    pub(crate) fn restored_state(&self) -> RestoredState {
        RestoredState {
            policy: PolicyFlags {
                queued: self.queued,
                held_for_disk: self.held_for_disk,
                share_paused: self.share_paused,
            },
            share_limits: self.share_limits,
            uploaded_bytes: self.uploaded_bytes,
            queue_position: self.queue_position,
            force_start: self.force_start,
        }
    }

    #[allow(dead_code)]
    pub fn category(&self) -> Option<&str> {
        self.category.as_deref()
    }

    #[allow(dead_code)]
    pub fn set_category(&mut self, category: Option<String>) {
        self.category = category;
    }

    pub fn into_add_torrent(self) -> anyhow::Result<(AddTorrent<'static>, AddTorrentOptions)> {
        let category = self.category.clone();
        let tags = self.tags.iter().cloned().collect();
        let part_files = self.part_files;
        let trackers = self.trackers.into_iter().collect::<Vec<_>>();
        let add_torrent = if !self.torrent_bytes.is_empty() {
            AddTorrent::TorrentFileBytes(self.torrent_bytes)
        } else {
            let magnet =
                Magnet::from_id20(self.info_hash, trackers.clone(), self.only_files.clone())
                    .to_string();
            AddTorrent::from_url(magnet)
        };

        let output_folder_root = self
            .output_folder_root
            .as_ref()
            .map(|path| path.to_str().context("broken root path"))
            .transpose()?
            .map(str::to_owned);
        let output_folder = if output_folder_root.is_some() {
            None
        } else {
            Some(
                self.output_folder
                    .to_str()
                    .context("broken path")?
                    .to_owned(),
            )
        };
        let opts = AddTorrentOptions {
            paused: self.is_paused,
            part_files,
            output_folder,
            output_folder_root,
            only_files: self.only_files,
            overwrite: true,
            category,
            tags,
            added_on: self.added_on,
            completion_on: self.completion_on,
            trackers: Some(trackers),
            ..Default::default()
        };

        Ok((add_torrent, opts))
    }
}

/// Per-torrent state that the session reapplies after adding a torrent back.
pub(crate) struct RestoredState {
    policy: PolicyFlags,
    share_limits: ShareLimits,
    uploaded_bytes: u64,
    queue_position: Option<i64>,
    force_start: bool,
}

impl RestoredState {
    pub(crate) fn apply(self, shared: &ManagedTorrentShared) {
        self.policy.store(shared);
        *shared.share_limits.write() = self.share_limits;
        shared
            .uploaded_bytes_base
            .store(self.uploaded_bytes, std::sync::atomic::Ordering::Relaxed);
        if let Some(position) = self.queue_position {
            shared
                .queue_position
                .store(position, std::sync::atomic::Ordering::Relaxed);
        }
        shared
            .force_start
            .store(self.force_start, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The persisted fields that come from the shared state of a live handle.
pub(crate) fn runtime_fields(
    torrent: &ManagedTorrentHandle,
) -> (PolicyFlags, ShareLimits, u64, Option<bool>) {
    (
        PolicyFlags::read(torrent.shared()),
        *torrent.shared().share_limits.read(),
        torrent.uploaded_bytes(),
        Some(torrent.shared().options.part_files),
    )
}

// TODO: make this info_hash first, ID-second.
#[async_trait]
pub trait SessionPersistenceStore: core::fmt::Debug + Send + Sync + BitVFactory {
    async fn next_id(&self) -> anyhow::Result<TorrentId>;
    async fn store(&self, id: TorrentId, torrent: &ManagedTorrentHandle) -> anyhow::Result<()>;
    async fn delete(&self, id: TorrentId) -> anyhow::Result<()>;
    async fn get(&self, id: TorrentId) -> anyhow::Result<SerializedTorrent>;
    async fn update_metadata(
        &self,
        id: TorrentId,
        torrent: &ManagedTorrentHandle,
    ) -> anyhow::Result<()>;
    async fn stream_all(
        &self,
    ) -> anyhow::Result<BoxStream<'_, anyhow::Result<(TorrentId, SerializedTorrent)>>>;

    async fn load_categories(&self) -> anyhow::Result<HashMap<String, TorrentCategory>> {
        Ok(HashMap::new())
    }

    async fn store_categories(
        &self,
        _categories: &HashMap<String, TorrentCategory>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn load_tags(&self) -> anyhow::Result<BTreeSet<String>> {
        Ok(BTreeSet::new())
    }

    async fn store_tags(&self, _tags: &BTreeSet<String>) -> anyhow::Result<()> {
        Ok(())
    }

    async fn load_session_settings(&self) -> anyhow::Result<SessionSettings> {
        Ok(SessionSettings::default())
    }

    async fn store_session_settings(&self, _settings: &SessionSettings) -> anyhow::Result<()> {
        Ok(())
    }
}

fn serialize_info_hash<S>(id: &Id20, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    id.as_string().serialize(serializer)
}

fn deserialize_info_hash<'de, D>(deserializer: D) -> Result<Id20, D::Error>
where
    D: Deserializer<'de>,
{
    Id20::deserialize(deserializer)
}
