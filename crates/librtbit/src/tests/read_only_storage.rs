use std::time::Duration;

use anyhow::Context;
use tempfile::TempDir;
use tokio::time::timeout;

use crate::{
    AddTorrent, AddTorrentOptions, CreateTorrentOptions, Session, SessionOptions,
    SessionPersistenceConfig, create_torrent,
    spawn_utils::BlockingSpawner,
    storage::{StorageFactoryExt, filesystem::FilesystemStorageFactory},
    tests::{mock_tracker::MockUdpTracker, test_util::wait_until},
};

async fn create_read_only_session(
    output_dir: &std::path::Path,
) -> anyhow::Result<std::sync::Arc<Session>> {
    Session::new_with_opts(
        output_dir.to_owned(),
        SessionOptions {
            disable_dht: true,
            disable_trackers: true,
            disable_local_service_discovery: true,
            default_storage_factory: Some(FilesystemStorageFactory::read_only().boxed()),
            ..Default::default()
        },
    )
    .await
}

async fn create_single_file_torrent(payload_dir: &std::path::Path) -> anyhow::Result<Vec<u8>> {
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

async fn create_multi_file_torrent(payload_dir: &std::path::Path) -> anyhow::Result<Vec<u8>> {
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

#[tokio::test(flavor = "multi_thread")]
async fn read_only_session_verifies_existing_payload_without_changing_it() -> anyhow::Result<()> {
    let payload_dir = TempDir::with_prefix("rtbit_read_only_existing")?;
    let payload_path = payload_dir.path().join("payload.bin");
    let payload = vec![0x5a; 128 * 1024];
    std::fs::write(&payload_path, &payload)?;
    let torrent = create_single_file_torrent(payload_dir.path()).await?;

    let session_dir = TempDir::with_prefix("rtbit_read_only_session")?;
    let session = create_read_only_session(session_dir.path()).await?;
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder: Some(payload_dir.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a torrent handle")?;

    timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .context("timed out waiting for read-only initialization")??;

    let stats = handle.stats();
    assert!(stats.finished);
    assert_eq!(stats.progress_bytes, stats.total_bytes);
    assert_eq!(std::fs::read(&payload_path)?, payload);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn read_only_session_does_not_create_missing_payload() -> anyhow::Result<()> {
    let source_dir = TempDir::with_prefix("rtbit_read_only_source")?;
    let source_path = source_dir.path().join("payload.bin");
    std::fs::write(&source_path, vec![0xa5; 64 * 1024])?;
    let torrent = create_single_file_torrent(source_dir.path()).await?;

    let destination_root = TempDir::with_prefix("rtbit_read_only_missing")?;
    let output_dir = destination_root.path().join("missing-output");
    let payload_path = output_dir.join("payload.bin");
    assert!(!output_dir.exists());

    let session_dir = TempDir::with_prefix("rtbit_read_only_session")?;
    let session = create_read_only_session(session_dir.path()).await?;
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder: Some(output_dir.to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a torrent handle")?;

    timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .context("timed out waiting for read-only initialization")??;

    let stats = handle.stats();
    assert!(!stats.finished);
    assert_eq!(stats.progress_bytes, 0);
    assert!(!output_dir.exists());
    assert!(!payload_path.exists());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn read_only_session_verifies_qbittorrent_multi_file_save_path() -> anyhow::Result<()> {
    let payload_root = TempDir::with_prefix("rtbit_qbit_multi_root")?;
    let torrent_root = payload_root.path().join("multi-file-payload");
    std::fs::create_dir(&torrent_root)?;
    std::fs::write(torrent_root.join("first.bin"), vec![0x12; 48 * 1024])?;
    std::fs::write(torrent_root.join("second.bin"), vec![0x34; 32 * 1024])?;
    let torrent = create_multi_file_torrent(&torrent_root).await?;

    let persistence_dir = TempDir::with_prefix("rtbit_qbit_multi_session")?;
    let new_session = || {
        Session::new_with_opts(
            payload_root.path().to_owned(),
            SessionOptions {
                disable_dht: true,
                disable_trackers: true,
                disable_local_service_discovery: true,
                default_storage_factory: Some(FilesystemStorageFactory::read_only().boxed()),
                persistence: Some(SessionPersistenceConfig::Json {
                    folder: Some(persistence_dir.path().to_owned()),
                }),
                ..Default::default()
            },
        )
    };

    let session = new_session().await?;
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder_root: Some(payload_root.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a torrent handle")?;
    timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .context("timed out waiting for qBittorrent-layout initialization")??;
    assert!(handle.stats().finished);
    assert_eq!(handle.shared().options.output_folder, torrent_root);
    assert_eq!(
        handle.shared().options.output_folder_root.as_deref(),
        Some(payload_root.path())
    );

    let info_hash = handle.info_hash();
    drop(handle);
    drop(session);
    let restored = new_session().await?;
    let restored_handle = restored
        .get(info_hash.into())
        .context("expected persisted torrent")?;
    timeout(
        Duration::from_secs(10),
        restored_handle.wait_until_initialized(),
    )
    .await
    .context("timed out waiting for restored qBittorrent-layout initialization")??;
    assert!(restored_handle.stats().finished);
    assert_eq!(restored_handle.shared().options.output_folder, torrent_root);
    assert_eq!(
        restored_handle
            .shared()
            .options
            .output_folder_root
            .as_deref(),
        Some(payload_root.path())
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn read_only_session_verifies_single_entry_multi_file_save_path() -> anyhow::Result<()> {
    let payload_root = TempDir::with_prefix("rtbit_qbit_single_entry_root")?;
    let torrent_root = payload_root.path().join("single-entry-payload");
    std::fs::create_dir(&torrent_root)?;
    std::fs::write(torrent_root.join("only.bin"), vec![0x56; 48 * 1024])?;
    let torrent = create_multi_file_torrent(&torrent_root).await?;

    let session_dir = TempDir::with_prefix("rtbit_qbit_single_entry_session")?;
    let session = create_read_only_session(session_dir.path()).await?;
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder_root: Some(payload_root.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a torrent handle")?;

    timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .context("timed out waiting for single-entry multi-file initialization")??;

    assert!(handle.stats().finished);
    assert_eq!(handle.shared().options.output_folder, torrent_root);
    assert_eq!(
        handle.shared().options.output_folder_root.as_deref(),
        Some(payload_root.path())
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn force_recheck_hashes_payload_and_keeps_paused_torrent_paused() -> anyhow::Result<()> {
    let payload_dir = TempDir::with_prefix("rtbit_recheck_paused_payload")?;
    let payload_path = payload_dir.path().join("payload.bin");
    std::fs::write(&payload_path, vec![0x6a; 64 * 1024])?;
    let torrent = create_single_file_torrent(payload_dir.path()).await?;

    let session_dir = TempDir::with_prefix("rtbit_recheck_paused_session")?;
    let session = create_read_only_session(session_dir.path()).await?;
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder: Some(payload_dir.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a torrent handle")?;
    timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .context("timed out waiting for initial validation")??;
    assert!(handle.stats().finished);

    std::fs::write(&payload_path, vec![0x7b; 64 * 1024])?;
    session.force_recheck(&handle).await?;
    assert!(matches!(
        handle.stats().state,
        crate::torrent_state::TorrentStatsState::Initializing
    ));
    timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .context("timed out waiting for forced recheck")??;

    let stats = handle.stats();
    assert!(matches!(
        stats.state,
        crate::torrent_state::TorrentStatsState::Paused
    ));
    assert!(!stats.finished);
    assert_eq!(stats.progress_bytes, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn force_recheck_resumes_torrent_that_was_live() -> anyhow::Result<()> {
    let payload_dir = TempDir::with_prefix("rtbit_recheck_live_payload")?;
    std::fs::write(
        payload_dir.path().join("payload.bin"),
        vec![0x8c; 64 * 1024],
    )?;
    let torrent = create_single_file_torrent(payload_dir.path()).await?;

    let session_dir = TempDir::with_prefix("rtbit_recheck_live_session")?;
    let session = create_read_only_session(session_dir.path()).await?;
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                overwrite: true,
                output_folder: Some(payload_dir.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a torrent handle")?;
    timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .context("timed out waiting for initial validation")??;
    assert!(matches!(
        handle.stats().state,
        crate::torrent_state::TorrentStatsState::Live
    ));

    session.force_recheck(&handle).await?;
    timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .context("timed out waiting for forced recheck")??;
    assert!(matches!(
        handle.stats().state,
        crate::torrent_state::TorrentStatsState::Live
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn added_trackers_are_validated_deduplicated_and_restored() -> anyhow::Result<()> {
    let payload_dir = TempDir::with_prefix("rtbit_tracker_restore_payload")?;
    std::fs::write(
        payload_dir.path().join("payload.bin"),
        vec![0x9d; 32 * 1024],
    )?;
    let torrent = create_single_file_torrent(payload_dir.path()).await?;

    let persistence_dir = TempDir::with_prefix("rtbit_tracker_restore_session")?;
    let new_session = || {
        Session::new_with_opts(
            payload_dir.path().to_owned(),
            SessionOptions {
                disable_dht: true,
                disable_trackers: true,
                disable_local_service_discovery: true,
                default_storage_factory: Some(FilesystemStorageFactory::read_only().boxed()),
                persistence: Some(SessionPersistenceConfig::Json {
                    folder: Some(persistence_dir.path().to_owned()),
                }),
                ..Default::default()
            },
        )
    };

    let session = new_session().await?;
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder: Some(payload_dir.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a torrent handle")?;
    timeout(Duration::from_secs(10), handle.wait_until_initialized()).await??;
    assert!(handle.shared().trackers.read().is_empty());

    let empty = session.add_trackers(&handle, Vec::new()).await;
    assert!(empty.is_err());
    assert!(handle.shared().trackers.read().is_empty());
    let invalid = session
        .add_trackers(&handle, vec!["not a tracker".to_string()])
        .await;
    assert!(invalid.is_err());
    assert!(handle.shared().trackers.read().is_empty());
    let unsupported = session
        .add_trackers(
            &handle,
            vec!["ftp://tracker.example.test/announce".to_string()],
        )
        .await;
    assert!(unsupported.is_err());
    assert!(handle.shared().trackers.read().is_empty());

    let tracker = "udp://tracker.example.test:6969/announce".to_string();
    session
        .add_trackers(&handle, vec![tracker.clone(), tracker.clone()])
        .await?;
    assert_eq!(handle.shared().trackers.read().len(), 1);
    assert_eq!(handle.shared().tracker_status.snapshot().len(), 1);

    let info_hash = handle.info_hash();
    drop(handle);
    drop(session);

    let restored = new_session().await?;
    let restored_handle = restored
        .get(info_hash.into())
        .context("expected persisted torrent")?;
    timeout(
        Duration::from_secs(10),
        restored_handle.wait_until_initialized(),
    )
    .await??;
    let restored_trackers = restored_handle.shared().trackers.read();
    assert_eq!(restored_trackers.len(), 1);
    assert!(restored_trackers.iter().any(|url| url.as_str() == tracker));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn adding_tracker_to_live_torrent_starts_announcing() -> anyhow::Result<()> {
    let tracker = MockUdpTracker::start().await;
    let payload_dir = TempDir::with_prefix("rtbit_tracker_live_payload")?;
    std::fs::write(
        payload_dir.path().join("payload.bin"),
        vec![0xae; 32 * 1024],
    )?;
    let torrent = create_single_file_torrent(payload_dir.path()).await?;
    let session_dir = TempDir::with_prefix("rtbit_tracker_live_session")?;
    let session = Session::new_with_opts(
        session_dir.path().to_owned(),
        SessionOptions {
            disable_dht: true,
            disable_local_service_discovery: true,
            ..Default::default()
        },
    )
    .await?;
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                overwrite: true,
                output_folder: Some(payload_dir.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a torrent handle")?;
    timeout(Duration::from_secs(10), handle.wait_until_initialized()).await??;

    session.add_trackers(&handle, vec![tracker.url()]).await?;
    wait_until(
        || {
            anyhow::ensure!(
                tracker.stats().announces > 0,
                "tracker has not received announce"
            );
            Ok(())
        },
        Duration::from_secs(5),
    )
    .await?;

    tracker.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn reannounce_and_removal_drive_live_tracker_loop() -> anyhow::Result<()> {
    let tracker = MockUdpTracker::start().await;
    let payload_dir = TempDir::with_prefix("rtbit_tracker_remove_payload")?;
    std::fs::write(
        payload_dir.path().join("payload.bin"),
        vec![0xae; 32 * 1024],
    )?;
    let torrent = create_single_file_torrent(payload_dir.path()).await?;
    let session_dir = TempDir::with_prefix("rtbit_tracker_remove_session")?;
    let session = Session::new_with_opts(
        session_dir.path().to_owned(),
        SessionOptions {
            disable_dht: true,
            disable_local_service_discovery: true,
            ..Default::default()
        },
    )
    .await?;
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                overwrite: true,
                output_folder: Some(payload_dir.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a torrent handle")?;
    timeout(Duration::from_secs(10), handle.wait_until_initialized()).await??;

    session.add_trackers(&handle, vec![tracker.url()]).await?;
    wait_until(
        || {
            anyhow::ensure!(tracker.stats().announces > 0, "no first announce");
            Ok(())
        },
        Duration::from_secs(5),
    )
    .await?;

    // The mock asks for a 5s interval; a forced reannounce must not wait it out.
    let before = tracker.stats().announces;
    assert!(handle.reannounce());
    wait_until(
        || {
            anyhow::ensure!(tracker.stats().announces > before, "no reannounce");
            Ok(())
        },
        Duration::from_secs(2),
    )
    .await?;

    let removed = session.remove_trackers(&handle, &[tracker.url()]).await?;
    assert_eq!(removed, 1);
    assert!(handle.shared().trackers.read().is_empty());
    assert!(handle.shared().tracker_status.snapshot().is_empty());

    // Past one full interval (and a reannounce), the removed URL stays quiet.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let after_remove = tracker.stats().announces;
    handle.reannounce();
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(tracker.stats().announces, after_remove);

    tracker.shutdown().await;
    Ok(())
}

/// Path, length, permissions-independent content hash of every entry under
/// `root`, so a test can assert the tree was left byte-for-byte alone.
fn snapshot_tree(root: &std::path::Path) -> Vec<(std::path::PathBuf, Option<u64>, Vec<u8>)> {
    fn walk(
        root: &std::path::Path,
        dir: &std::path::Path,
        out: &mut Vec<(std::path::PathBuf, Option<u64>, Vec<u8>)>,
    ) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path.strip_prefix(root).unwrap().to_owned();
            if path.is_dir() {
                out.push((rel, None, Vec::new()));
                walk(root, &path, out);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                out.push((rel, Some(bytes.len() as u64), bytes));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Mimic a `:ro` bind mount where the platform allows it: directories and
/// files lose their write bits. (Root ignores the bits; the tree snapshot is
/// the real guard.)
#[cfg(unix)]
fn set_tree_read_only(root: &std::path::Path, read_only: bool) {
    use std::os::unix::fs::PermissionsExt;
    let mode = |is_dir: bool| match (is_dir, read_only) {
        (true, true) => 0o555,
        (false, true) => 0o444,
        (true, false) => 0o755,
        (false, false) => 0o644,
    };
    let mut stack = vec![root.to_owned()];
    let mut dirs = Vec::new();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode(false)))
                    .unwrap();
            }
        }
        dirs.push(dir);
    }
    for dir in dirs {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode(true))).unwrap();
    }
}

#[cfg(not(unix))]
fn set_tree_read_only(_root: &std::path::Path, _read_only: bool) {}

/// A three-file payload (complete, short, missing) for a torrent created from
/// the full data, laid out under `<root>/payload`.
async fn make_damaged_multi_file_payload(root: &std::path::Path) -> anyhow::Result<Vec<u8>> {
    let payload = root.join("payload");
    std::fs::create_dir(&payload)?;
    std::fs::write(payload.join("complete.bin"), vec![0x11; 48 * 1024])?;
    std::fs::write(payload.join("short.bin"), vec![0x22; 40 * 1024])?;
    std::fs::write(payload.join("missing.bin"), vec![0x33; 32 * 1024])?;
    let torrent = create_multi_file_torrent(&payload).await?;
    // Damage the payload after the torrent was made from the full data.
    std::fs::write(payload.join("short.bin"), vec![0x22; 10 * 1024])?;
    std::fs::remove_file(payload.join("missing.bin"))?;
    Ok(torrent)
}

/// Gap 2: with read-only storage (`RTBIT_STORAGE_READ_ONLY=true`) over a
/// read-only tree, a full recheck of a damaged payload never truncates,
/// preallocates or creates a file or directory.
#[tokio::test(flavor = "multi_thread")]
async fn read_only_full_recheck_never_truncates_preallocates_or_creates_files() -> anyhow::Result<()>
{
    let root = TempDir::with_prefix("rtbit_read_only_recheck_tree")?;
    let torrent = make_damaged_multi_file_payload(root.path()).await?;
    let payload = root.path().join("payload");
    let before = snapshot_tree(root.path());
    set_tree_read_only(root.path(), true);

    let session_dir = TempDir::with_prefix("rtbit_read_only_recheck_session")?;
    let session = create_read_only_session(session_dir.path()).await?;
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder: Some(payload.to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a torrent handle")?;
    timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .context("timed out waiting for initial validation")??;
    let first = handle.stats();

    session.force_recheck(&handle).await?;
    timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .context("timed out waiting for full recheck")??;
    let stats = handle.stats();

    set_tree_read_only(root.path(), false);
    assert_eq!(
        snapshot_tree(root.path()),
        before,
        "recheck changed the tree"
    );
    assert!(!payload.join("missing.bin").exists());
    assert_eq!(
        std::fs::metadata(payload.join("short.bin"))?.len(),
        10 * 1024
    );
    assert!(!stats.finished);
    // Only the complete file's pieces verify, and the recheck agrees with the
    // add-time check.
    assert_eq!(stats.progress_bytes, first.progress_bytes);
    assert!(stats.progress_bytes > 0 && stats.progress_bytes < stats.total_bytes);
    Ok(())
}

/// Gap 3: a torrent whose payload is missing or short on read-only storage is
/// kept stopped and flagged, even when added running or resumed, and never
/// goes live to download.
#[tokio::test(flavor = "multi_thread")]
async fn read_only_incomplete_payload_stays_stopped_and_flagged() -> anyhow::Result<()> {
    let root = TempDir::with_prefix("rtbit_read_only_incomplete_tree")?;
    let torrent = make_damaged_multi_file_payload(root.path()).await?;
    let payload = root.path().join("payload");
    let before = snapshot_tree(root.path());

    let session_dir = TempDir::with_prefix("rtbit_read_only_incomplete_session")?;
    let session = create_read_only_session(session_dir.path()).await?;
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                // Added running: the read-only guard, not the caller, keeps it stopped.
                paused: false,
                overwrite: true,
                output_folder: Some(payload.to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a torrent handle")?;
    timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .context("timed out waiting for initial validation")??;

    let assert_stopped_and_flagged = |what: &str| {
        let stats = handle.stats();
        assert!(
            matches!(stats.state, crate::torrent_state::TorrentStatsState::Paused),
            "{what}: expected paused, got {:?}",
            stats.state
        );
        assert!(handle.is_paused(), "{what}: not marked paused");
        assert!(!stats.finished, "{what}");
        assert_eq!(
            stats.error.as_deref(),
            Some(crate::torrent_state::READ_ONLY_INCOMPLETE_PAYLOAD),
            "{what}: not flagged"
        );
        assert!(stats.live.is_none(), "{what}: went live");
    };
    assert_stopped_and_flagged("after add");

    // A resume does not take it live either.
    session.unpause(&handle).await?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_stopped_and_flagged("after resume");

    // Nor does a recheck of a torrent the user wanted running.
    session.force_recheck(&handle).await?;
    timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .context("timed out waiting for recheck")??;
    assert_stopped_and_flagged("after recheck");

    assert_eq!(snapshot_tree(root.path()), before);
    Ok(())
}

/// A complete payload on read-only storage is not flagged and can seed.
#[tokio::test(flavor = "multi_thread")]
async fn read_only_complete_payload_is_not_flagged_and_seeds() -> anyhow::Result<()> {
    let payload_dir = TempDir::with_prefix("rtbit_read_only_complete")?;
    std::fs::write(
        payload_dir.path().join("payload.bin"),
        vec![0x44; 64 * 1024],
    )?;
    let torrent = create_single_file_torrent(payload_dir.path()).await?;

    let session_dir = TempDir::with_prefix("rtbit_read_only_complete_session")?;
    let session = create_read_only_session(session_dir.path()).await?;
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                overwrite: true,
                output_folder: Some(payload_dir.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a torrent handle")?;
    timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .context("timed out waiting for validation")??;
    let stats = handle.stats();
    assert!(matches!(
        stats.state,
        crate::torrent_state::TorrentStatsState::Live
    ));
    assert!(stats.finished);
    assert!(stats.error.is_none());
    Ok(())
}
