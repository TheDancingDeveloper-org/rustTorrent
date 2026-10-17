pub mod json;
#[cfg(feature = "postgres")]
pub mod postgres;

use std::{collections::HashMap, collections::HashSet, path::PathBuf};

use anyhow::Context;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use librtbit_core::Id20;
use librtbit_core::magnet::Magnet;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    AddTorrent, AddTorrentOptions, bitv_factory::BitVFactory, category::TorrentCategory,
    session::PolicyFlags, session::TorrentId, torrent_state::ManagedTorrentHandle,
};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_folder: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_folder: Option<PathBuf>,
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
}

impl SerializedTorrent {
    pub fn info_hash(&self) -> &Id20 {
        &self.info_hash
    }

    /// The policy flags to reapply once the torrent is added back.
    pub(crate) fn policy_flags(&self) -> PolicyFlags {
        PolicyFlags {
            queued: self.queued,
            held_for_disk: self.held_for_disk,
            share_paused: self.share_paused,
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
            output_folder,
            output_folder_root,
            only_files: self.only_files,
            overwrite: true,
            category,
            added_on: self.added_on,
            completion_on: self.completion_on,
            trackers: Some(trackers),
            ..Default::default()
        };

        Ok((add_torrent, opts))
    }
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
