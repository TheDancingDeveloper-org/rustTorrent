use std::{any::TypeId, collections::HashMap, path::PathBuf};

use crate::{
    api::TorrentIdOrHash,
    bitv::{BitV, DiskBackedBitV},
    bitv_factory::BitVFactory,
    category::TorrentCategory,
    session::TorrentId,
    spawn_utils::BlockingSpawner,
    storage::filesystem::FilesystemStorageFactory,
    torrent_state::ManagedTorrentHandle,
    type_aliases::BF,
};
use anyhow::{Context, bail};
use async_trait::async_trait;
use futures::{StreamExt, stream::BoxStream};
use itertools::Itertools;
use librtbit_core::Id20;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, trace, warn};

use super::{SerializedTorrent, SessionPersistenceStore, SessionSettings};
use crate::session::PolicyFlags;

#[derive(Serialize, Deserialize, Default)]
struct SerializedSessionDatabase {
    torrents: HashMap<usize, SerializedTorrent>,
    #[serde(default)]
    settings: SessionSettings,
}

pub struct JsonSessionPersistenceStore {
    output_folder: PathBuf,
    db_filename: PathBuf,
    db_content: tokio::sync::RwLock<SerializedSessionDatabase>,
    spawner: BlockingSpawner,
}

impl std::fmt::Debug for JsonSessionPersistenceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JSON database: {:?}", self.output_folder)
    }
}

impl JsonSessionPersistenceStore {
    pub async fn new(output_folder: PathBuf, spawner: BlockingSpawner) -> anyhow::Result<Self> {
        let db_filename = output_folder.join("session.json");
        tokio::fs::create_dir_all(&output_folder)
            .await
            .with_context(|| {
                format!("couldn't create directory {output_folder:?} for session storage")
            })?;

        let db = match tokio::fs::File::open(&db_filename).await {
            Ok(f) => {
                let mut buf = Vec::new();
                let mut rdr = tokio::io::BufReader::new(f);
                rdr.read_to_end(&mut buf).await?;

                serde_json::from_reader(&buf[..]).context("error deserializing session database")?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Default::default(),
            Err(e) => {
                return Err(e).context(format!("error opening session file {db_filename:?}"));
            }
        };

        Ok(Self {
            db_filename,
            output_folder,
            db_content: tokio::sync::RwLock::new(db),
            spawner,
        })
    }

    async fn to_hash(&self, id: TorrentIdOrHash) -> anyhow::Result<Id20> {
        match id {
            TorrentIdOrHash::Id(id) => self
                .db_content
                .read()
                .await
                .torrents
                .get(&id)
                .map(|v| *v.info_hash())
                .context("not found"),
            TorrentIdOrHash::Hash(h) => Ok(h),
        }
    }

    async fn flush(&self) -> anyhow::Result<()> {
        // we don't need the write lock technically, but we need to stop concurrent modifications
        let db_content = self.db_content.write().await;
        let tmp_filename = format!("{}.tmp", self.db_filename.to_str().unwrap());
        let mut tmp = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp_filename)
            .await
            .with_context(|| format!("error opening {tmp_filename:?}"))?;
        trace!(?tmp_filename, "opened temp file");

        let mut buf = Vec::new();
        serde_json::to_writer(&mut buf, &*db_content).context("error serializing")?;

        trace!(?tmp_filename, "serialized DB as JSON");
        tmp.write_all(&buf)
            .await
            .with_context(|| format!("error writing {tmp_filename:?}"))?;
        trace!(?tmp_filename, "wrote to temp file");

        tokio::fs::rename(&tmp_filename, &self.db_filename)
            .await
            .context("error renaming persistence file")?;
        trace!(filename=?self.db_filename, "wrote persistence");
        Ok(())
    }

    fn torrent_bytes_filename(&self, info_hash: &Id20) -> PathBuf {
        self.output_folder.join(format!("{info_hash:?}.torrent"))
    }

    fn bitv_filename(&self, info_hash: &Id20) -> PathBuf {
        self.output_folder.join(format!("{info_hash:?}.bitv"))
    }

    fn categories_filename(&self) -> PathBuf {
        self.output_folder.join("categories.json")
    }

    async fn update_db(
        &self,
        id: TorrentId,
        torrent: &ManagedTorrentHandle,
        write_torrent_file: bool,
    ) -> anyhow::Result<()> {
        if !torrent
            .shared
            .storage_factory
            .is_type_id(TypeId::of::<FilesystemStorageFactory>())
        {
            bail!("storages other than FilesystemStorageFactory are not supported");
        }

        // Read the torrent and store it under one lock. Otherwise two updates can read
        // before they lock and store in the reverse order, and the older state wins.
        let mut db_content = self.db_content.write().await;
        let policy = PolicyFlags::read(torrent.shared());
        let st = SerializedTorrent {
            trackers: torrent
                .shared()
                .trackers
                .read()
                .iter()
                .map(|u| u.to_string())
                .collect(),
            info_hash: torrent.info_hash(),
            // we don't serialize this here, but to a file instead.
            torrent_bytes: Default::default(),
            only_files: torrent.only_files().clone(),
            is_paused: torrent.is_paused(),
            queued: policy.queued,
            held_for_disk: policy.held_for_disk,
            share_paused: policy.share_paused,
            output_folder: torrent.shared().options.output_folder.clone(),
            output_folder_root: torrent.shared().options.output_folder_root.clone(),
            category: torrent.shared().category.read().clone(),
            added_on: Some(torrent.shared().added_on),
            completion_on: match torrent
                .shared()
                .completion_on
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                0 => None,
                value => Some(value),
            },
        };

        let torrent_bytes = torrent
            .metadata
            .load()
            .as_ref()
            .map(|i| i.torrent_bytes.clone())
            .unwrap_or_default();

        if write_torrent_file && !torrent_bytes.is_empty() {
            let torrent_bytes_file = self.torrent_bytes_filename(&torrent.info_hash());
            match tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&torrent_bytes_file)
                .await
            {
                Ok(mut f) => {
                    if let Err(e) = f.write_all(&torrent_bytes).await {
                        warn!(error=?e, file=?torrent_bytes_file, "error writing torrent bytes")
                    }
                }
                Err(e) => {
                    warn!(error=?e, file=?torrent_bytes_file, "error opening torrent bytes file")
                }
            }
        }

        db_content.torrents.insert(id, st);
        // flush() takes the same lock, so release it first.
        drop(db_content);
        self.flush().await?;

        Ok(())
    }
}

#[async_trait::async_trait]
impl BitVFactory for JsonSessionPersistenceStore {
    async fn load(&self, id: TorrentIdOrHash) -> anyhow::Result<Option<Box<dyn BitV>>> {
        let h = self.to_hash(id).await?;
        let filename = self.bitv_filename(&h);
        match DiskBackedBitV::new(filename.clone(), self.spawner.clone()).await {
            Ok(bitv) => Ok(Some(bitv.into_dyn())),
            Err(e) => {
                if let Some(e) = e.downcast_ref::<std::io::Error>()
                    && matches!(e.kind(), std::io::ErrorKind::NotFound)
                {
                    return Ok(None);
                }
                // CRC mismatch or other corruption: log warning and return None
                // to trigger a full integrity check instead of aborting
                if format!("{e:#}").contains("CRC32 mismatch") {
                    warn!(?filename, "bitv CRC32 mismatch, will do full check");
                    return Ok(None);
                }
                return Err(e);
            }
        }
    }

    async fn clear(&self, id: TorrentIdOrHash) -> anyhow::Result<()> {
        let h = self.to_hash(id).await?;
        let filename = self.bitv_filename(&h);
        tokio::fs::remove_file(&filename)
            .await
            .with_context(|| format!("error removing {filename:?}"))
    }

    async fn store_initial_check(
        &self,
        id: TorrentIdOrHash,
        b: BF,
    ) -> anyhow::Result<Box<dyn BitV>> {
        let h = self.to_hash(id).await?;
        let filename = self.bitv_filename(&h);
        // Use a distinct .init_tmp suffix from the DiskBackedBitV flusher (which uses .bitv.tmp)
        // to avoid a race where an old flusher's rename steals our temp file before we rename it.
        let tmp_filename = format!("{}.init_tmp", filename.to_str().context("bug")?);
        let mut dst = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_filename)
            .await
            .with_context(|| format!("error opening {tmp_filename:?}"))?;
        let data = crate::bitv::bitv_with_header(b.as_raw_slice());
        dst.write_all(&data)
            .await
            .with_context(|| format!("error writing bitslice to {tmp_filename:?}"))?;
        tokio::fs::rename(&tmp_filename, &filename)
            .await
            .with_context(|| format!("error renaming {tmp_filename:?} to {filename:?}"))?;
        trace!(?filename, "stored initial check bitfield");
        Ok(DiskBackedBitV::new(filename.clone(), self.spawner.clone())
            .await
            .with_context(|| format!("error constructing MmapBitV from file {filename:?}"))?
            .into_dyn())
    }
}

#[async_trait]
impl SessionPersistenceStore for JsonSessionPersistenceStore {
    async fn next_id(&self) -> anyhow::Result<TorrentId> {
        Ok(self
            .db_content
            .read()
            .await
            .torrents
            .keys()
            .copied()
            .max()
            .map(|max| max + 1)
            .unwrap_or(0))
    }

    async fn delete(&self, id: TorrentId) -> anyhow::Result<()> {
        debug!(?id, "attempting to delete");
        // BIG NOTE: DO NOT inline this variable. Otherwise Rust doesn't drop the lock and it deadlocks
        // when calling flush - because let bindings prolong the duration.
        let removed = self.db_content.write().await.torrents.remove(&id);
        if let Some(t) = removed {
            debug!(?id, "deleted from in-memory db, flushing");
            self.flush().await?;
            for tf in [
                self.torrent_bytes_filename(&t.info_hash),
                self.bitv_filename(&t.info_hash),
            ] {
                if let Err(e) = tokio::fs::remove_file(&tf).await {
                    warn!(error=?e, filename=?tf, "error removing");
                } else {
                    debug!(filename=?tf, "removed");
                }
            }
        } else {
            bail!("error deleting: didn't find torrent id={id}")
        }

        Ok(())
    }

    async fn get(&self, id: TorrentId) -> anyhow::Result<SerializedTorrent> {
        let mut st = self
            .db_content
            .read()
            .await
            .torrents
            .get(&id)
            .cloned()
            .context("no torrent found")?;
        let mut buf = Vec::new();
        let torrent_bytes_filename = self.torrent_bytes_filename(&st.info_hash);
        let mut torrent_bytes_file = match tokio::fs::File::open(&torrent_bytes_filename).await {
            Ok(f) => f,
            Err(e) => {
                warn!(error=?e, filename=?torrent_bytes_filename, "error opening torrent bytes file");
                return Ok(st);
            }
        };
        if let Err(e) = torrent_bytes_file.read_to_end(&mut buf).await {
            warn!(error=?e, filename=?torrent_bytes_filename, "error reading torrent bytes file");
        } else {
            st.torrent_bytes = buf.into();
        }
        return Ok(st);
    }

    async fn stream_all(
        &self,
    ) -> anyhow::Result<BoxStream<'_, anyhow::Result<(TorrentId, SerializedTorrent)>>> {
        let all_ids = self
            .db_content
            .read()
            .await
            .torrents
            .keys()
            .copied()
            .collect_vec();
        Ok(futures::stream::iter(all_ids)
            .then(move |id| async move { self.get(id).await.map(move |st| (id, st)) })
            .boxed())
    }

    async fn store(&self, id: TorrentId, torrent: &ManagedTorrentHandle) -> anyhow::Result<()> {
        self.update_db(id, torrent, true).await
    }

    async fn update_metadata(
        &self,
        id: TorrentId,
        torrent: &ManagedTorrentHandle,
    ) -> anyhow::Result<()> {
        self.update_db(id, torrent, false).await
    }

    async fn load_session_settings(&self) -> anyhow::Result<SessionSettings> {
        Ok(self.db_content.read().await.settings.clone())
    }

    async fn store_session_settings(&self, settings: &SessionSettings) -> anyhow::Result<()> {
        self.db_content.write().await.settings = settings.clone();
        self.flush().await
    }

    async fn load_categories(&self) -> anyhow::Result<HashMap<String, TorrentCategory>> {
        let filename = self.categories_filename();
        match tokio::fs::File::open(&filename).await {
            Ok(f) => {
                let mut buf = Vec::new();
                let mut rdr = tokio::io::BufReader::new(f);
                rdr.read_to_end(&mut buf).await?;
                let categories: HashMap<String, TorrentCategory> =
                    serde_json::from_reader(&buf[..]).context("error deserializing categories")?;
                Ok(categories)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(e) => Err(e).context(format!("error opening categories file {filename:?}")),
        }
    }

    async fn store_categories(
        &self,
        categories: &HashMap<String, TorrentCategory>,
    ) -> anyhow::Result<()> {
        let filename = self.categories_filename();
        let tmp_filename = format!("{}.tmp", filename.to_str().unwrap());
        let mut tmp = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp_filename)
            .await
            .with_context(|| format!("error opening {tmp_filename:?}"))?;

        let mut buf = Vec::new();
        serde_json::to_writer_pretty(&mut buf, categories)
            .context("error serializing categories")?;

        tmp.write_all(&buf)
            .await
            .with_context(|| format!("error writing {tmp_filename:?}"))?;

        tokio::fs::rename(&tmp_filename, &filename)
            .await
            .context("error renaming categories file")?;
        trace!(filename=?filename, "wrote categories");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn json_store_persists_session_settings() {
        let dir = tempfile::tempdir().unwrap();
        let spawner = BlockingSpawner::new(1);
        let store = JsonSessionPersistenceStore::new(dir.path().to_path_buf(), spawner.clone())
            .await
            .unwrap();
        let settings = SessionSettings {
            output_folder: Some(dir.path().join("downloads/{category}")),
            completed_folder: Some(dir.path().join("completed/{category}")),
        };

        store.store_session_settings(&settings).await.unwrap();
        let reopened = JsonSessionPersistenceStore::new(dir.path().to_path_buf(), spawner)
            .await
            .unwrap();

        assert_eq!(reopened.load_session_settings().await.unwrap(), settings);
    }
}
