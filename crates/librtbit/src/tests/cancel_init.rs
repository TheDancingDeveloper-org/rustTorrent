use std::time::Duration;

use anyhow::Context;
use tempfile::TempDir;
use tokio::time::timeout;
use tracing::info;

use crate::{
    AddTorrent, AddTorrentOptions, CreateTorrentOptions, Session, SessionOptions,
    api::TorrentIdOrHash,
    create_torrent,
    spawn_utils::BlockingSpawner,
    tests::test_util::{create_default_random_dir_with_torrents, setup_test_logging},
    torrent_state::ManagedTorrentState,
};

/// Helper to create a session with DHT and listeners disabled (for fast, isolated tests).
async fn create_test_session(
    output_dir: &std::path::Path,
) -> anyhow::Result<std::sync::Arc<Session>> {
    Session::new_with_opts(
        output_dir.to_owned(),
        SessionOptions {
            disable_dht: true,
            persistence: None,
            disable_local_service_discovery: true,
            ..Default::default()
        },
    )
    .await
    .context("error creating test session")
}

/// Test that deleting a torrent while it's still initializing (checking files)
/// cancels the init task promptly via the per-torrent cancellation token.
#[tokio::test(flavor = "multi_thread")]
async fn test_delete_cancels_initializing_torrent() -> anyhow::Result<()> {
    setup_test_logging();

    // Create a relatively large torrent so initialization takes a non-trivial amount of time.
    let files_dir =
        create_default_random_dir_with_torrents(4, 1024 * 1024, Some("test_delete_init"));
    let torrent = create_torrent(
        files_dir.path(),
        CreateTorrentOptions {
            name: None,
            piece_length: Some(16384),
            ..Default::default()
        },
        &BlockingSpawner::new(1),
    )
    .await?;

    // Use a separate output directory so the torrent has to do a full initial check
    // (no existing files to fast-resume from).
    let output_dir = TempDir::with_prefix("test_delete_init_out")?;
    let session = create_test_session(output_dir.path()).await?;

    let response = session
        .add_torrent(
            AddTorrent::from_bytes(torrent.as_bytes()?),
            Some(AddTorrentOptions {
                paused: false,
                ..Default::default()
            }),
        )
        .await?;

    let (id, handle) = match response {
        crate::AddTorrentResponse::Added(id, h) => (id, h),
        other => anyhow::bail!(
            "unexpected response: expected Added, got {:?}",
            other.into_handle().is_some()
        ),
    };

    // Verify it's in Initializing state.
    let is_initializing = handle.with_state(|s| matches!(s, ManagedTorrentState::Initializing(_)));
    // It might have already finished init if it was very fast, so we just log it.
    info!("torrent is initializing: {is_initializing}");

    // Delete the torrent. This should cancel the init task and return promptly.
    let delete_result = timeout(
        Duration::from_secs(5),
        session.delete(TorrentIdOrHash::Id(id), false),
    )
    .await
    .context("delete() timed out - init task was not cancelled")?;

    delete_result.context("delete() returned error")?;

    // Verify the torrent was removed from the session.
    assert!(session.get(TorrentIdOrHash::Id(id)).is_none());

    info!("test_delete_cancels_initializing_torrent passed");
    Ok(())
}

/// Test that pausing a torrent during initialization cancels the init task
/// and transitions to a state that allows re-initialization later.
#[tokio::test(flavor = "multi_thread")]
async fn test_pause_during_initialization() -> anyhow::Result<()> {
    setup_test_logging();

    let files_dir =
        create_default_random_dir_with_torrents(4, 1024 * 1024, Some("test_pause_init"));
    let torrent = create_torrent(
        files_dir.path(),
        CreateTorrentOptions {
            name: None,
            piece_length: Some(16384),
            ..Default::default()
        },
        &BlockingSpawner::new(1),
    )
    .await?;

    let output_dir = TempDir::with_prefix("test_pause_init_out")?;
    let session = create_test_session(output_dir.path()).await?;

    let response = session
        .add_torrent(
            AddTorrent::from_bytes(torrent.as_bytes()?),
            Some(AddTorrentOptions {
                paused: false,
                ..Default::default()
            }),
        )
        .await?;

    let handle = response
        .into_handle()
        .context("expected a torrent handle")?;

    // Attempt to pause. With our changes, this should succeed even during initialization.
    let pause_result = session.pause(&handle).await;
    match pause_result {
        Ok(()) => {
            info!("pause succeeded during initialization");
            // After pause, the torrent should be in Error state
            // (with "paused during initialization" message) and paused flag set.
            assert!(handle.is_paused(), "torrent should be marked as paused");

            // Stopping the check leaves the torrent stopped with its check
            // deferred (or paused, if the check won the race), never errored.
            let state_name = handle.with_state(|s| s.name().to_string());
            info!("state after pause: {state_name} {:?}", handle.stats().error);
            assert_ne!(state_name, "error");
            assert!(matches!(
                handle.stats().state,
                crate::torrent_state::stats::TorrentStatsState::Paused
            ));

            // Starting it again runs the check and brings it up.
            session.unpause(&handle).await?;
            timeout(Duration::from_secs(30), handle.wait_until_initialized())
                .await
                .context("timed out re-initializing after stop")??;
            assert_ne!(handle.with_state(|s| s.name()), "error");
        }
        Err(e) => {
            // If the torrent already finished initialization, pause would succeed
            // from the Live state, or fail if it's already paused.
            info!("pause returned error (init may have completed): {e:#}");
        }
    }

    info!("test_pause_during_initialization passed");
    Ok(())
}

/// Test that the per-torrent cancellation token is properly triggered when
/// delete or forget is called, and that it's a child of the session token.
#[tokio::test(flavor = "multi_thread")]
async fn test_cancellation_token_propagation() -> anyhow::Result<()> {
    setup_test_logging();

    let files_dir = create_default_random_dir_with_torrents(1, 4096, Some("test_cancel_token"));
    let torrent = create_torrent(
        files_dir.path(),
        CreateTorrentOptions {
            name: None,
            piece_length: Some(1024),
            ..Default::default()
        },
        &BlockingSpawner::new(1),
    )
    .await?;

    let output_dir = TempDir::with_prefix("test_cancel_token_out")?;
    let session = create_test_session(output_dir.path()).await?;

    let response = session
        .add_torrent(
            AddTorrent::from_bytes(torrent.as_bytes()?),
            Some(AddTorrentOptions {
                paused: true,
                ..Default::default()
            }),
        )
        .await?;

    let (id, handle) = match response {
        crate::AddTorrentResponse::Added(id, h) => (id, h),
        other => anyhow::bail!("unexpected response: {:?}", other.into_handle().is_some()),
    };

    // Wait for initialization to complete (it's paused, so it will init then stay paused).
    timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .context("timed out waiting for initialization")?
        .context("error during initialization")?;

    // Capture the per-torrent cancellation token before deletion.
    let torrent_token = handle.shared.child_token();

    // Verify the torrent token is NOT cancelled before delete.
    assert!(
        !torrent_token.is_cancelled(),
        "torrent token should not be cancelled before delete"
    );

    // Delete the torrent.
    session
        .delete(TorrentIdOrHash::Id(id), false)
        .await
        .context("error deleting torrent")?;

    // After delete, the per-torrent token should be cancelled.
    // The child token we captured above should also be cancelled
    // (since we cancelled its parent).
    assert!(
        torrent_token.is_cancelled(),
        "torrent cancellation token should be cancelled after delete"
    );

    // Verify the torrent is no longer in the session.
    assert!(session.get(TorrentIdOrHash::Id(id)).is_none());

    // Also verify session-level token is NOT cancelled (only the torrent was removed).
    assert!(
        !session.cancellation_token().is_cancelled(),
        "session token should not be cancelled by single torrent delete"
    );

    info!("test_cancellation_token_propagation passed");
    Ok(())
}

/// Test that session shutdown cancels all per-torrent tokens (since they are children
/// of the session token).
#[tokio::test(flavor = "multi_thread")]
async fn test_session_shutdown_cancels_torrent_tokens() -> anyhow::Result<()> {
    setup_test_logging();

    let files_dir = create_default_random_dir_with_torrents(1, 4096, Some("test_session_shutdown"));
    let torrent = create_torrent(
        files_dir.path(),
        CreateTorrentOptions {
            name: None,
            piece_length: Some(1024),
            ..Default::default()
        },
        &BlockingSpawner::new(1),
    )
    .await?;

    let output_dir = TempDir::with_prefix("test_session_shutdown_out")?;
    let session = create_test_session(output_dir.path()).await?;

    let response = session
        .add_torrent(
            AddTorrent::from_bytes(torrent.as_bytes()?),
            Some(AddTorrentOptions {
                paused: true,
                ..Default::default()
            }),
        )
        .await?;

    let handle = response.into_handle().context("expected torrent handle")?;

    timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .context("timed out waiting for init")?
        .context("init error")?;

    // Capture a child of the per-torrent token.
    let torrent_child_token = handle.shared.child_token();
    assert!(!torrent_child_token.is_cancelled());

    // Stop the session.
    session.stop().await;

    // The session token should be cancelled.
    assert!(session.cancellation_token().is_cancelled());

    // The per-torrent token (child of session) should also be cancelled.
    assert!(
        torrent_child_token.is_cancelled(),
        "per-torrent token should be cancelled after session shutdown"
    );

    info!("test_session_shutdown_cancels_torrent_tokens passed");
    Ok(())
}
