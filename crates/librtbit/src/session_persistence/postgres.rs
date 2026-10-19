use std::{path::PathBuf, time::Duration};

use super::runtime_fields;
use crate::{
    api::TorrentIdOrHash, bitv::BitV, bitv_factory::BitVFactory, session::ShareLimits,
    session::TorrentId, torrent_state::ManagedTorrentHandle, type_aliases::BF,
};
use anyhow::Context;
use futures::{StreamExt, stream::BoxStream};
use librtbit_core::{Id20, spawn_utils::spawn};
use sqlx::{Pool, Postgres};
use tracing::debug_span;

use super::{SerializedTorrent, SessionPersistenceStore};

#[derive(Debug)]
pub struct PostgresSessionStorage {
    pool: Pool<Postgres>,
}

#[derive(sqlx::FromRow)]
struct TorrentsTableRecord {
    id: i32,
    info_hash: Vec<u8>,
    torrent_bytes: Vec<u8>,
    trackers: Vec<String>,
    output_folder: String,
    output_folder_root: Option<String>,
    only_files: Option<Vec<i32>>,
    is_paused: bool,
    queued: bool,
    held_for_disk: bool,
    share_paused: bool,
    share_max_ratio: Option<f64>,
    share_max_seeding_secs: Option<i64>,
    uploaded_bytes: i64,
    part_files: Option<bool>,
}

/// The four columns for share limits, upload total and `.part` choice, in the
/// order they are bound.
type RuntimeColumns = (Option<f64>, Option<i64>, i64, Option<bool>);

fn runtime_columns(torrent: &ManagedTorrentHandle) -> anyhow::Result<RuntimeColumns> {
    let (_, limits, uploaded, part_files) = runtime_fields(torrent);
    Ok((
        limits.max_ratio,
        limits
            .max_seeding_time
            .map(|d| i64::try_from(d.as_secs()))
            .transpose()?,
        i64::try_from(uploaded)?,
        part_files,
    ))
}

impl TorrentsTableRecord {
    fn into_serialized_torrent(self) -> Option<(TorrentId, SerializedTorrent)> {
        Some((
            self.id as TorrentId,
            SerializedTorrent {
                info_hash: Id20::from_bytes(&self.info_hash).ok()?,
                torrent_bytes: self.torrent_bytes.into(),
                trackers: self.trackers.into_iter().collect(),
                output_folder: PathBuf::from(self.output_folder),
                output_folder_root: self.output_folder_root.map(PathBuf::from),
                only_files: self
                    .only_files
                    .map(|v| v.into_iter().map(|v| v as usize).collect()),
                is_paused: self.is_paused,
                category: None,
                tags: Default::default(),
                added_on: None,
                completion_on: None,
                queued: self.queued,
                held_for_disk: self.held_for_disk,
                share_paused: self.share_paused,
                queue_position: None,
                force_start: false,
                share_limits: ShareLimits {
                    max_ratio: self.share_max_ratio,
                    max_seeding_time: self
                        .share_max_seeding_secs
                        .map(|s| Duration::from_secs(s.max(0) as u64)),
                },
                uploaded_bytes: self.uploaded_bytes.max(0) as u64,
                part_files: self.part_files,
            },
        ))
    }
}

impl PostgresSessionStorage {
    pub async fn new(connection_string: &str) -> anyhow::Result<Self> {
        use sqlx::postgres::PgPoolOptions;

        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(connection_string)
            .await?;

        macro_rules! exec {
            ($q:expr) => {
                sqlx::query($q)
                    .execute(&pool)
                    .await
                    .context($q)
                    .context("error running query")?;
            };
        }

        exec!("CREATE SEQUENCE IF NOT EXISTS torrents_id AS integer;");

        exec!(
            "CREATE TABLE IF NOT EXISTS torrents (
          id INTEGER PRIMARY KEY DEFAULT nextval('torrents_id'),
          info_hash BYTEA NOT NULL,
          torrent_bytes BYTEA NOT NULL,
          trackers TEXT[] NOT NULL,
          output_folder TEXT NOT NULL,
          output_folder_root TEXT,
          only_files INTEGER[],
          is_paused BOOLEAN NOT NULL
        )"
        );

        exec!("ALTER TABLE torrents ADD COLUMN IF NOT EXISTS have_bitfield BYTEA");
        exec!("ALTER TABLE torrents ADD COLUMN IF NOT EXISTS output_folder_root TEXT");
        exec!(
            "ALTER TABLE torrents ADD COLUMN IF NOT EXISTS queued BOOLEAN NOT NULL DEFAULT FALSE"
        );
        exec!(
            "ALTER TABLE torrents ADD COLUMN IF NOT EXISTS held_for_disk BOOLEAN NOT NULL DEFAULT FALSE"
        );
        exec!(
            "ALTER TABLE torrents ADD COLUMN IF NOT EXISTS share_paused BOOLEAN NOT NULL DEFAULT FALSE"
        );
        exec!("ALTER TABLE torrents ADD COLUMN IF NOT EXISTS share_max_ratio DOUBLE PRECISION");
        exec!("ALTER TABLE torrents ADD COLUMN IF NOT EXISTS share_max_seeding_secs BIGINT");
        exec!(
            "ALTER TABLE torrents ADD COLUMN IF NOT EXISTS uploaded_bytes BIGINT NOT NULL DEFAULT 0"
        );
        exec!("ALTER TABLE torrents ADD COLUMN IF NOT EXISTS part_files BOOLEAN");

        Ok(Self { pool })
    }
}

#[async_trait::async_trait]
impl SessionPersistenceStore for PostgresSessionStorage {
    async fn next_id(&self) -> anyhow::Result<TorrentId> {
        let (id,): (i32,) = sqlx::query_as("SELECT nextval('torrents_id')::int")
            .fetch_one(&self.pool)
            .await
            .context("error executing SELECT nextval")?;
        Ok(id as usize)
    }

    async fn store(&self, id: TorrentId, torrent: &ManagedTorrentHandle) -> anyhow::Result<()> {
        let torrent_bytes = torrent
            .metadata
            .load()
            .as_ref()
            .map(|i| i.torrent_bytes.clone())
            .unwrap_or_default();
        let trackers = torrent
            .shared()
            .trackers
            .read()
            .iter()
            .map(|tracker| tracker.to_string())
            .collect::<Vec<_>>();
        let (share_max_ratio, share_max_seeding_secs, uploaded_bytes, part_files) =
            runtime_columns(torrent)?;
        let q = "INSERT INTO torrents (id, info_hash, torrent_bytes, trackers, output_folder, output_folder_root, only_files, is_paused, share_max_ratio, share_max_seeding_secs, uploaded_bytes, part_files)
        VALUES($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        ON CONFLICT(id) DO NOTHING";
        sqlx::query(q)
            .bind::<i32>(id.try_into()?)
            .bind(&torrent.info_hash().0[..])
            .bind(torrent_bytes.as_ref())
            .bind(trackers)
            .bind(
                torrent
                    .shared()
                    .options
                    .output_folder
                    .to_str()
                    .context("output_folder")?
                    .to_owned(),
            )
            .bind(
                torrent
                    .shared()
                    .options
                    .output_folder_root
                    .as_ref()
                    .map(|path| path.to_str().context("output_folder_root"))
                    .transpose()?,
            )
            .bind(torrent.only_files().map(|o| {
                o.into_iter()
                    .filter_map(|o| o.try_into().ok())
                    .collect::<Vec<i32>>()
            }))
            .bind(torrent.is_paused())
            .bind(share_max_ratio)
            .bind(share_max_seeding_secs)
            .bind(uploaded_bytes)
            .bind(part_files)
            .execute(&self.pool)
            .await
            .context("error executing INSERT INTO torrents")?;
        Ok(())
    }

    async fn delete(&self, id: TorrentId) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM torrents WHERE id = $1")
            .bind::<i32>(id.try_into()?)
            .execute(&self.pool)
            .await
            .context("error executing DELETE FROM torrents")?;
        Ok(())
    }

    async fn get(&self, id: TorrentId) -> anyhow::Result<SerializedTorrent> {
        let row = sqlx::query_as::<_, TorrentsTableRecord>("SELECT * FROM torrents WHERE id = ?")
            .bind::<i32>(id.try_into()?)
            .fetch_one(&self.pool)
            .await
            .context("error executing SELECT * FROM torrents")?;
        row.into_serialized_torrent()
            .context("bug")
            .map(|(_, st)| st)
    }

    async fn update_metadata(
        &self,
        id: TorrentId,
        torrent: &ManagedTorrentHandle,
    ) -> anyhow::Result<()> {
        let trackers = torrent
            .shared()
            .trackers
            .read()
            .iter()
            .map(|tracker| tracker.to_string())
            .collect::<Vec<_>>();
        let (policy, _, _, _) = runtime_fields(torrent);
        let (share_max_ratio, share_max_seeding_secs, uploaded_bytes, part_files) =
            runtime_columns(torrent)?;
        sqlx::query(
            "UPDATE torrents SET trackers = $1, only_files = $2, is_paused = $3, queued = $4, held_for_disk = $5, share_paused = $6, share_max_ratio = $7, share_max_seeding_secs = $8, uploaded_bytes = $9, part_files = $10 WHERE id = $11",
        )
        .bind(trackers)
        .bind(torrent.only_files().map(|v| {
            v.into_iter()
                .filter_map(|f| f.try_into().ok())
                .collect::<Vec<i32>>()
        }))
        .bind(torrent.is_paused())
        .bind(policy.queued)
        .bind(policy.held_for_disk)
        .bind(policy.share_paused)
        .bind(share_max_ratio)
        .bind(share_max_seeding_secs)
        .bind(uploaded_bytes)
        .bind(part_files)
        .bind::<i32>(id.try_into()?)
        .execute(&self.pool)
        .await
        .context("error executing UPDATE torrents")?;
        Ok(())
    }

    async fn stream_all(
        &self,
    ) -> anyhow::Result<BoxStream<'_, anyhow::Result<(TorrentId, SerializedTorrent)>>> {
        let torrents = sqlx::query_as::<_, TorrentsTableRecord>("SELECT * FROM torrents")
            .fetch_all(&self.pool)
            .await
            .context("error executing SELECT * FROM torrents")?
            .into_iter()
            .filter_map(TorrentsTableRecord::into_serialized_torrent)
            .map(Ok);
        Ok(futures::stream::iter(torrents).boxed())
    }
}

struct PgBitfield {
    torrent_id: TorrentIdOrHash,
    inmem: BF,
    pool: Pool<Postgres>,
}

impl BitV for PgBitfield {
    fn as_slice(&self) -> &bitvec::prelude::BitSlice<u8, bitvec::prelude::Msb0> {
        self.inmem.as_bitslice()
    }

    fn as_slice_mut(&mut self) -> &mut bitvec::prelude::BitSlice<u8, bitvec::prelude::Msb0> {
        self.inmem.as_mut_bitslice()
    }

    fn into_dyn(self) -> Box<dyn BitV> {
        Box::new(self)
    }

    fn as_bytes(&self) -> &[u8] {
        self.inmem.as_raw_slice()
    }

    fn flush(&mut self, _flush_async: bool) -> anyhow::Result<()> {
        // TODO: make flush async, and don't spawn this, to avoid allocations and capture the result.
        spawn(
            debug_span!("pg_update_bitfield", id=?self.torrent_id),
            "pg_update_bitfield",
            {
                let hb = self.as_bytes().to_owned();
                let pool = self.pool.clone();
                let torrent_id = self.torrent_id;

                macro_rules! exec {
                    ($q:expr, $bf:expr, $id:expr) => {
                        sqlx::query($q)
                            .bind($bf)
                            .bind($id)
                            .execute(&pool)
                            .await
                            .context($q)
                            .context("error executing query")
                    };
                }

                async move {
                    match torrent_id {
                        TorrentIdOrHash::Id(id) => {
                            let id: i32 = id.try_into()?;
                            exec!(
                                "UPDATE torrents SET have_bitfield = $1 WHERE id = $2",
                                &hb,
                                id
                            )?;
                        }
                        TorrentIdOrHash::Hash(h) => {
                            exec!(
                                "UPDATE torrents SET have_bitfield = $1 WHERE info_hash = $2",
                                &hb,
                                &h.0[..]
                            )?;
                        }
                    };
                    Ok::<_, anyhow::Error>(())
                }
            },
        );
        Ok(())
    }
}

#[async_trait::async_trait]
impl BitVFactory for PostgresSessionStorage {
    async fn load(&self, id: TorrentIdOrHash) -> anyhow::Result<Option<Box<dyn BitV>>> {
        #[derive(sqlx::FromRow)]
        struct HaveBitfield {
            have_bitfield: Option<Vec<u8>>,
        }

        macro_rules! exec {
            ($q:expr, $v:expr) => {
                sqlx::query_as($q)
                    .bind($v)
                    .fetch_one(&self.pool)
                    .await
                    .context($q)
                    .context("error executing query")?
            };
        }

        let hb: HaveBitfield = match id {
            TorrentIdOrHash::Id(id) => {
                let id: i32 = id.try_into()?;
                exec!("SELECT have_bitfield FROM torrents WHERE id = $1", id)
            }
            TorrentIdOrHash::Hash(h) => {
                exec!(
                    "SELECT have_bitfield FROM torrents WHERE info_hash = $1",
                    &h.0[..]
                )
            }
        };

        let hb = hb.have_bitfield;
        Ok(hb.map(|b| {
            PgBitfield {
                torrent_id: id,
                inmem: BF::from_boxed_slice(b.into_boxed_slice()),
                pool: self.pool.clone(),
            }
            .into_dyn()
        }))
    }

    async fn store_initial_check(
        &self,
        id: TorrentIdOrHash,
        b: BF,
    ) -> anyhow::Result<Box<dyn BitV>> {
        let mut bf = PgBitfield {
            torrent_id: id,
            inmem: b,
            pool: self.pool.clone(),
        };
        bf.flush(false)?;
        Ok(bf.into_dyn())
    }

    async fn clear(&self, id: TorrentIdOrHash) -> anyhow::Result<()> {
        macro_rules! exec {
            ($q:expr, $v:expr) => {
                sqlx::query($q)
                    .bind($v)
                    .execute(&self.pool)
                    .await
                    .context($q)
                    .context("error executing query")?
            };
        }

        match id {
            TorrentIdOrHash::Id(id) => {
                let id: i32 = id.try_into()?;
                exec!("UPDATE torrents SET have_bitfield = NULL WHERE id = $1", id);
            }
            TorrentIdOrHash::Hash(h) => {
                exec!(
                    "UPDATE torrents SET have_bitfield = NULL WHERE info_hash = $1",
                    &h.0[..]
                );
            }
        }
        Ok(())
    }
}
