//! Session-level tests for the policy pass: download queueing, the low-disk
//! guard and share limits. The restart tests check that the queued, held and
//! share-paused state written by the pass comes back when a session restarts.

use std::{path::Path, sync::Arc, sync::atomic::Ordering};

use anyhow::Context;
use tempfile::TempDir;
use tokio::time::{Duration, timeout};

use crate::{
    AddTorrent, AddTorrentOptions, CreateTorrentOptions, Session, SessionOptions,
    api::TorrentIdOrHash,
    create_torrent,
    session::{SessionPersistenceConfig, ShareLimits},
    spawn_utils::BlockingSpawner,
    tests::test_util::{create_default_random_dir_with_torrents, setup_test_logging},
    torrent_state::ManagedTorrentHandle,
};

async fn create_torrent_bytes(payload_dir: &Path) -> anyhow::Result<Vec<u8>> {
    let torrent = create_torrent(
        payload_dir,
        CreateTorrentOptions {
            piece_length: Some(16_384),
            ..Default::default()
        },
        &BlockingSpawner::new(1),
    )
    .await?;
    Ok(torrent.as_bytes()?.to_vec())
}

/// A torrent over fresh random content. Each call has its own info hash, so
/// downloads from separate calls are distinct torrents.
async fn random_torrent(prefix: &str) -> anyhow::Result<Vec<u8>> {
    let payload = create_default_random_dir_with_torrents(2, 64 * 1024, Some(prefix));
    create_torrent_bytes(payload.path()).await
}

/// Start a session. With `persist` set, state is written to that folder so a
/// later session can restore it.
async fn create_session(output_dir: &Path, persist: Option<&Path>) -> anyhow::Result<Arc<Session>> {
    Session::new_with_opts(
        output_dir.to_owned(),
        SessionOptions {
            disable_dht: true,
            disable_dht_persistence: true,
            disable_local_service_discovery: true,
            persistence: persist.map(|folder| SessionPersistenceConfig::Json {
                folder: Some(folder.to_owned()),
            }),
            ..Default::default()
        },
    )
    .await
    .context("error creating test session")
}

/// Add a torrent that downloads into `output_dir`, which holds no files, and
/// wait for it to initialize.
async fn add_download(
    session: &Arc<Session>,
    torrent: Vec<u8>,
    output_dir: &Path,
) -> anyhow::Result<ManagedTorrentHandle> {
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                overwrite: true,
                output_folder: Some(output_dir.to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a torrent handle")?;
    timeout(Duration::from_secs(10), handle.wait_until_initialized()).await??;
    Ok(handle)
}

fn handle_by_id(session: &Session, id: usize) -> anyhow::Result<ManagedTorrentHandle> {
    session
        .get(TorrentIdOrHash::Id(id))
        .with_context(|| format!("torrent {id} was not restored"))
}

/// Two downloads, a queue limit of one, then the limit lifted: the newer
/// torrent waits in the queue and starts once a slot is free.
#[tokio::test(flavor = "multi_thread")]
async fn queue_limit_parks_newer_download_until_slot_frees() -> anyhow::Result<()> {
    setup_test_logging();

    let root = TempDir::with_prefix("policy_queue_out")?;
    let session = create_session(root.path(), None).await?;
    let first = add_download(
        &session,
        random_torrent("policy_queue_a").await?,
        &root.path().join("first"),
    )
    .await?;
    let second = add_download(
        &session,
        random_torrent("policy_queue_b").await?,
        &root.path().join("second"),
    )
    .await?;

    session.set_max_active_downloads(Some(1));
    session.policy_pass().await;

    assert!(!first.is_paused(), "oldest download keeps its slot");
    assert!(!first.is_queued());
    assert!(second.is_paused(), "newer download waits in the queue");
    assert!(second.is_queued());

    session.set_max_active_downloads(None);
    session.policy_pass().await;

    assert!(
        !second.is_paused(),
        "lifting the limit starts the queued download"
    );
    assert!(!second.is_queued());

    session.stop().await;
    Ok(())
}

/// A queued download is still queued after a restart, and the next policy pass
/// starts it. Before the flags were persisted, it came back paused with no
/// reason and never started.
#[tokio::test(flavor = "multi_thread")]
async fn queued_download_stays_queued_across_restart() -> anyhow::Result<()> {
    setup_test_logging();

    let root = TempDir::with_prefix("policy_restart_out")?;
    let persist = TempDir::with_prefix("policy_restart_state")?;

    let session = create_session(root.path(), Some(persist.path())).await?;
    let first = add_download(
        &session,
        random_torrent("policy_restart_a").await?,
        &root.path().join("first"),
    )
    .await?;
    let second = add_download(
        &session,
        random_torrent("policy_restart_b").await?,
        &root.path().join("second"),
    )
    .await?;
    let (first_id, second_id) = (first.id(), second.id());

    session.set_max_active_downloads(Some(1));
    session.policy_pass().await;
    assert!(second.is_queued());
    session.stop().await;
    drop(session);

    let session = create_session(root.path(), Some(persist.path())).await?;
    let restored_first = handle_by_id(&session, first_id)?;
    let restored_second = handle_by_id(&session, second_id)?;

    assert!(
        restored_second.is_paused() && restored_second.is_queued(),
        "queued state must survive the restart"
    );
    assert!(!restored_first.is_queued());

    session.set_max_active_downloads(Some(1));
    session.policy_pass().await;
    assert!(
        restored_second.is_queued(),
        "the first download still holds the only slot"
    );

    session.set_max_active_downloads(None);
    session.policy_pass().await;
    assert!(
        !restored_second.is_paused(),
        "the restored queued download starts once the limit is lifted"
    );

    session.stop().await;
    Ok(())
}

/// A download held by the low-disk guard stays held across a restart, and is
/// released once free space recovers in the new session.
#[tokio::test(flavor = "multi_thread")]
async fn disk_hold_survives_restart_and_releases() -> anyhow::Result<()> {
    setup_test_logging();

    let payload = create_default_random_dir_with_torrents(2, 64 * 1024, Some("policy_hold"));
    let torrent = create_torrent_bytes(payload.path()).await?;

    let root = TempDir::with_prefix("policy_hold_out")?;
    let persist = TempDir::with_prefix("policy_hold_state")?;

    let session = create_session(root.path(), Some(persist.path())).await?;
    let download = add_download(&session, torrent, &root.path().join("dl")).await?;
    let id = download.id();

    // Any real filesystem has less than u64::MAX bytes free, so this holds.
    session.set_min_free_disk_space(Some(u64::MAX));
    session.policy_pass().await;
    assert!(download.is_paused());
    assert!(download.shared().held_for_disk.load(Ordering::Relaxed));
    session.stop().await;
    drop(session);

    let session = create_session(root.path(), Some(persist.path())).await?;
    let restored = handle_by_id(&session, id)?;
    assert!(restored.is_paused(), "held download comes back paused");
    assert!(
        restored.shared().held_for_disk.load(Ordering::Relaxed),
        "the hold must survive the restart"
    );

    // A threshold of 1 byte is always cleared, so the guard releases the download.
    session.set_min_free_disk_space(Some(1));
    session.policy_pass().await;
    assert!(
        !restored.is_paused(),
        "free space recovered, download resumes"
    );
    assert!(!restored.shared().held_for_disk.load(Ordering::Relaxed));

    session.stop().await;
    Ok(())
}

/// A seed that hits its share limit stays paused across a restart, and the
/// policy pass does not resume it.
#[tokio::test(flavor = "multi_thread")]
async fn share_paused_seed_stays_paused_across_restart() -> anyhow::Result<()> {
    setup_test_logging();

    // The payload is also the output folder, so the torrent starts out finished.
    let payload = create_default_random_dir_with_torrents(2, 64 * 1024, Some("policy_share"));
    let torrent = create_torrent_bytes(payload.path()).await?;

    let root = TempDir::with_prefix("policy_share_out")?;
    let persist = TempDir::with_prefix("policy_share_state")?;

    let session = create_session(root.path(), Some(persist.path())).await?;
    let seed = session
        .add_torrent(
            AddTorrent::from_bytes(torrent.clone()),
            Some(AddTorrentOptions {
                overwrite: true,
                output_folder: Some(payload.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a torrent handle")?;
    timeout(Duration::from_secs(10), seed.wait_until_initialized()).await??;
    anyhow::ensure!(seed.stats().finished, "seed should start out finished");
    let id = seed.id();

    // Any upload ratio reaches zero, so the limit trips at once.
    session.set_share_limits(
        &seed,
        ShareLimits {
            max_ratio: Some(0.0),
            max_seeding_time: None,
        },
    );
    session.policy_pass().await;
    assert!(seed.is_paused());
    assert!(seed.shared().share_paused.load(Ordering::Relaxed));
    session.stop().await;
    drop(session);

    let session = create_session(root.path(), Some(persist.path())).await?;
    let restored = handle_by_id(&session, id)?;
    assert!(restored.is_paused(), "share-paused seed comes back paused");
    assert!(
        restored.shared().share_paused.load(Ordering::Relaxed),
        "the share-limit reason must survive the restart"
    );

    session.policy_pass().await;
    assert!(
        restored.is_paused(),
        "the policy pass must not resume a share-paused seed"
    );

    session.stop().await;
    Ok(())
}
