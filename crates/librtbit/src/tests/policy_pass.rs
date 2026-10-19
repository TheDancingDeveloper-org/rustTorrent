//! Session-level tests for the policy pass: download queueing, the low-disk
//! guard and share limits. The restart tests check that the queued, held and
//! share-paused state written by the pass comes back when a session restarts.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
};

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
    session
        .set_share_limits(
            &seed,
            ShareLimits {
                max_ratio: Some(0.0),
                max_seeding_time: None,
                max_inactive_seeding_time: None,
            },
        )
        .await;
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

    // A user resume clears the reason and restarts the inactivity clock.
    *restored.shared().upload_activity.lock() = (42, 1);
    session.unpause(&restored).await?;
    assert!(!restored.shared().share_paused.load(Ordering::Relaxed));
    assert_eq!(*restored.shared().upload_activity.lock(), (0, 0));

    session.stop().await;
    Ok(())
}

/// Every file under `dir`, at any depth.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(files_under(&path));
        } else {
            out.push(path);
        }
    }
    out
}

fn is_part_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "part")
}

/// Share limits and the upload total survive a restart, and the restored limit
/// still trips once uploads continue past it.
#[tokio::test(flavor = "multi_thread")]
async fn share_limits_and_uploads_survive_restart() -> anyhow::Result<()> {
    setup_test_logging();

    let payload =
        create_default_random_dir_with_torrents(2, 64 * 1024, Some("policy_share_restart"));
    let torrent = create_torrent_bytes(payload.path()).await?;
    let root = TempDir::with_prefix("policy_share_restart_out")?;
    let persist = TempDir::with_prefix("policy_share_restart_state")?;
    let limits = ShareLimits {
        max_ratio: Some(1.5),
        max_seeding_time: Some(Duration::from_secs(3600)),
        max_inactive_seeding_time: Some(Duration::from_secs(900)),
    };

    let session = create_session(root.path(), Some(persist.path())).await?;
    let seed = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
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
    let total = seed.stats().total_bytes;

    session.set_share_limits(&seed, limits).await;
    // One full copy uploaded so far: ratio 1.0, under the 1.5 limit.
    seed.shared()
        .uploaded_bytes_base
        .store(total, Ordering::Relaxed);
    session.stop().await;
    drop(session);

    let session = create_session(root.path(), Some(persist.path())).await?;
    let restored = handle_by_id(&session, id)?;
    timeout(Duration::from_secs(10), restored.wait_until_initialized()).await??;
    assert_eq!(
        session.share_limits(&restored),
        limits,
        "limits must survive the restart"
    );
    assert_eq!(
        restored.uploaded_bytes(),
        total,
        "the upload total must survive the restart"
    );

    session.policy_pass().await;
    assert!(
        !restored.is_paused(),
        "a seed at ratio 1.0 is under its 1.5 limit"
    );

    // Uploads continue after the restart. The restored total counts toward the
    // limit, so the seed reaches ratio 2.0 and the restored limit trips.
    restored
        .shared()
        .uploaded_bytes_base
        .fetch_add(total, Ordering::Relaxed);
    session.policy_pass().await;
    assert!(
        restored.is_paused(),
        "the restored ratio limit must be enforced"
    );
    assert!(restored.shared().share_paused.load(Ordering::Relaxed));

    session.stop().await;
    Ok(())
}

/// A torrent that was storing `.part` files keeps doing so after a restart even
/// when the session setting is off. Its partial data is left intact.
#[tokio::test(flavor = "multi_thread")]
async fn part_file_choice_survives_restart() -> anyhow::Result<()> {
    setup_test_logging();

    const SIZE: usize = 64 * 1024;
    let root = TempDir::with_prefix("policy_part_out")?;
    let persist = TempDir::with_prefix("policy_part_state")?;
    let output = root.path().join("dl");

    let session = create_session(root.path(), Some(persist.path())).await?;
    session.set_use_part_files(true);
    let download = add_download(&session, random_torrent("policy_part").await?, &output).await?;
    let id = download.id();
    assert!(download.shared().options.part_files);

    let parts: Vec<PathBuf> = files_under(&output)
        .into_iter()
        .filter(|p| is_part_file(p))
        .collect();
    anyhow::ensure!(
        !parts.is_empty(),
        "a .part download should create .part files"
    );
    for part in &parts {
        std::fs::write(part, vec![7u8; SIZE])?;
    }
    session.stop().await;
    drop(session);

    // The session setting is off, so only the persisted choice can keep `.part`.
    let session = create_session(root.path(), Some(persist.path())).await?;
    let restored = handle_by_id(&session, id)?;
    timeout(Duration::from_secs(10), restored.wait_until_initialized()).await??;

    assert!(
        restored.shared().options.part_files,
        "the restored torrent keeps .part"
    );
    let final_files: Vec<PathBuf> = files_under(&output)
        .into_iter()
        .filter(|p| !is_part_file(p))
        .collect();
    assert!(
        final_files.is_empty(),
        "no final-name file may be created: {final_files:?}"
    );
    for part in &parts {
        assert_eq!(
            std::fs::read(part)?,
            vec![7u8; SIZE],
            "partial data must not be thrown away"
        );
    }

    session.stop().await;
    Ok(())
}

/// An existing `.part` file is adopted on restore even when the torrent was saved
/// without `.part` and the session setting is off, so its data is not orphaned.
#[tokio::test(flavor = "multi_thread")]
async fn existing_part_file_is_adopted_with_switch_off() -> anyhow::Result<()> {
    setup_test_logging();

    const SIZE: usize = 64 * 1024;
    let root = TempDir::with_prefix("policy_adopt_out")?;
    let persist = TempDir::with_prefix("policy_adopt_state")?;
    let output = root.path().join("dl");

    let session = create_session(root.path(), Some(persist.path())).await?;
    let download = add_download(&session, random_torrent("policy_adopt").await?, &output).await?;
    let id = download.id();
    assert!(!download.shared().options.part_files);
    session.stop().await;
    drop(session);

    // Simulate an interrupted `.part` download: the data sits under `.part` names
    // and no final-name file exists.
    let finals: Vec<PathBuf> = files_under(&output)
        .into_iter()
        .filter(|p| !is_part_file(p))
        .collect();
    anyhow::ensure!(!finals.is_empty(), "the first run should create files");
    let mut parts = Vec::new();
    for file in &finals {
        std::fs::write(file, vec![9u8; SIZE])?;
        let part = PathBuf::from(format!("{}.part", file.display()));
        std::fs::rename(file, &part)?;
        parts.push(part);
    }

    let session = create_session(root.path(), Some(persist.path())).await?;
    let restored = handle_by_id(&session, id)?;
    timeout(Duration::from_secs(10), restored.wait_until_initialized()).await??;

    let final_files: Vec<PathBuf> = files_under(&output)
        .into_iter()
        .filter(|p| !is_part_file(p))
        .collect();
    assert!(
        final_files.is_empty(),
        "the .part data must be adopted, not replaced: {final_files:?}"
    );
    for part in &parts {
        assert_eq!(
            std::fs::read(part)?,
            vec![9u8; SIZE],
            "adopted .part data must be kept"
        );
    }

    session.stop().await;
    Ok(())
}

/// A session that saves torrent state every `interval`. Cancelling `kill` stops
/// the session's tasks without a final save, as a killed process would.
async fn create_session_saving_every(
    output_dir: &Path,
    persist: &Path,
    interval: Duration,
    kill: tokio_util::sync::CancellationToken,
) -> anyhow::Result<Arc<Session>> {
    Session::new_with_opts(
        output_dir.to_owned(),
        SessionOptions {
            disable_dht: true,
            disable_dht_persistence: true,
            disable_local_service_discovery: true,
            persistence: Some(SessionPersistenceConfig::Json {
                folder: Some(persist.to_owned()),
            }),
            persist_interval: Some(interval),
            cancellation_token: Some(kill),
            ..Default::default()
        },
    )
    .await
    .context("error creating test session")
}

/// A finished seed in `payload`, added to `session`, with the given share limits.
async fn add_finished_seed(
    session: &Arc<Session>,
    payload: &Path,
    limits: ShareLimits,
) -> anyhow::Result<ManagedTorrentHandle> {
    let torrent = create_torrent_bytes(payload).await?;
    let seed = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                overwrite: true,
                output_folder: Some(payload.to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a torrent handle")?;
    timeout(Duration::from_secs(10), seed.wait_until_initialized()).await??;
    anyhow::ensure!(seed.stats().finished, "seed should start out finished");
    session.set_share_limits(&seed, limits).await;
    Ok(seed)
}

/// Upload totals reach disk through the periodic save when the session is dropped
/// without a clean stop. The total is set after the last save, so only the timer
/// can have written it.
#[tokio::test(flavor = "multi_thread")]
async fn upload_total_saved_by_timer_survives_drop_without_stop() -> anyhow::Result<()> {
    setup_test_logging();

    let payload = create_default_random_dir_with_torrents(2, 64 * 1024, Some("policy_timer"));
    let root = TempDir::with_prefix("policy_timer_out")?;
    let persist = TempDir::with_prefix("policy_timer_state")?;
    let limits = ShareLimits {
        max_ratio: Some(3.0),
        max_seeding_time: None,
        max_inactive_seeding_time: None,
    };

    let kill = tokio_util::sync::CancellationToken::new();
    let session = create_session_saving_every(
        payload.path(),
        persist.path(),
        Duration::from_millis(100),
        kill.clone(),
    )
    .await?;
    let seed = add_finished_seed(&session, payload.path(), limits).await?;
    let id = seed.id();
    let total = seed.stats().total_bytes;

    // Recorded after the save that set the limits, so only the timer can persist it.
    seed.shared()
        .uploaded_bytes_base
        .store(total, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(500)).await;
    // Kill without stop(): the session's tasks end with no final save.
    kill.cancel();
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(seed);
    drop(session);

    let session = create_session(root.path(), Some(persist.path())).await?;
    let restored = handle_by_id(&session, id)?;
    timeout(Duration::from_secs(10), restored.wait_until_initialized()).await??;
    assert_eq!(
        restored.uploaded_bytes(),
        total,
        "the upload total must reach disk through the timer"
    );

    session.stop().await;
    Ok(())
}

/// Share limits set with `set_share_limits` are saved at once, so they survive a
/// drop without a clean stop even when the periodic save never runs.
#[tokio::test(flavor = "multi_thread")]
async fn share_limits_saved_on_set_survive_drop_without_stop() -> anyhow::Result<()> {
    setup_test_logging();

    let payload = create_default_random_dir_with_torrents(2, 64 * 1024, Some("policy_setlimits"));
    let root = TempDir::with_prefix("policy_setlimits_out")?;
    let persist = TempDir::with_prefix("policy_setlimits_state")?;
    let limits = ShareLimits {
        max_ratio: Some(2.5),
        max_seeding_time: Some(Duration::from_secs(7200)),
        max_inactive_seeding_time: None,
    };

    // An hour between timer saves, so the save in `set_share_limits` is the only one.
    let kill = tokio_util::sync::CancellationToken::new();
    let session = create_session_saving_every(
        payload.path(),
        persist.path(),
        Duration::from_secs(3600),
        kill.clone(),
    )
    .await?;
    let seed = add_finished_seed(&session, payload.path(), limits).await?;
    let id = seed.id();
    kill.cancel();
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(seed);
    drop(session);

    let session = create_session(root.path(), Some(persist.path())).await?;
    let restored = handle_by_id(&session, id)?;
    timeout(Duration::from_secs(10), restored.wait_until_initialized()).await??;
    assert_eq!(
        session.share_limits(&restored),
        limits,
        "limits set on the session must be saved without a stop"
    );

    session.stop().await;
    Ok(())
}

/// Moving a waiting download to the top of the queue gives it the slot, and
/// the queue order survives a restart.
#[tokio::test(flavor = "multi_thread")]
async fn queue_moves_reassign_slots_and_persist() -> anyhow::Result<()> {
    setup_test_logging();

    let root = TempDir::with_prefix("policy_order_out")?;
    let persist = TempDir::with_prefix("policy_order_state")?;
    let session = create_session(root.path(), Some(persist.path())).await?;
    let first = add_download(
        &session,
        random_torrent("policy_order_a").await?,
        &root.path().join("first"),
    )
    .await?;
    let second = add_download(
        &session,
        random_torrent("policy_order_b").await?,
        &root.path().join("second"),
    )
    .await?;
    let (first_id, second_id) = (first.id(), second.id());

    session.set_queue_limits(Some(1), None).await?;
    session.policy_pass().await;
    assert!(second.is_queued());

    session
        .move_in_queue(&[second_id], crate::session::QueueMove::Top)
        .await;
    assert_eq!(session.queue_order(), vec![second_id, first_id]);
    session.policy_pass().await;
    assert!(!second.is_paused(), "the moved download takes the slot");
    assert!(
        first.is_queued(),
        "the other download goes back to the queue"
    );

    session.stop().await;
    drop(session);

    let session = create_session(root.path(), Some(persist.path())).await?;
    assert_eq!(session.max_active_downloads(), Some(1), "limit persisted");
    assert_eq!(session.queue_order(), vec![second_id, first_id]);
    assert!(handle_by_id(&session, first_id)?.is_queued());

    session.stop().await;
    Ok(())
}

/// A force-started download runs outside the queue limit and takes no slot;
/// a user pause clears force-start.
#[tokio::test(flavor = "multi_thread")]
async fn force_start_bypasses_queue_limit() -> anyhow::Result<()> {
    setup_test_logging();

    let root = TempDir::with_prefix("policy_force_out")?;
    let session = create_session(root.path(), None).await?;
    let first = add_download(
        &session,
        random_torrent("policy_force_a").await?,
        &root.path().join("first"),
    )
    .await?;
    let second = add_download(
        &session,
        random_torrent("policy_force_b").await?,
        &root.path().join("second"),
    )
    .await?;

    session.set_max_active_downloads(Some(1));
    session.policy_pass().await;
    assert!(second.is_queued());

    session.set_force_start(&second, true).await?;
    assert!(second.is_force_started());
    assert!(!second.is_paused() && !second.is_queued());
    session.policy_pass().await;
    assert!(!second.is_paused(), "force-started download is not demoted");
    assert!(!first.is_paused(), "and it does not take the queued slot");

    session.pause(&second).await?;
    assert!(
        !second.is_force_started(),
        "a user pause clears force-start"
    );

    session.stop().await;
    Ok(())
}
