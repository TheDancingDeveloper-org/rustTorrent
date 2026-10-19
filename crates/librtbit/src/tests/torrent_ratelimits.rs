//! Per-torrent speed limits survive a session restart.

use std::num::NonZeroU32;

use anyhow::Context;
use tempfile::TempDir;

use crate::{
    AddTorrent, AddTorrentOptions, Api, Session, SessionOptions, TorrentRateLimits,
    api::TorrentIdOrHash,
    session::SessionPersistenceConfig,
    tests::test_util::{create_default_random_dir_with_torrents, setup_test_logging},
};

async fn create_session(
    output: &std::path::Path,
    persist: &std::path::Path,
) -> anyhow::Result<Api> {
    let session = Session::new_with_opts(
        output.to_owned(),
        SessionOptions {
            disable_dht: true,
            disable_dht_persistence: true,
            disable_local_service_discovery: true,
            persistence: Some(SessionPersistenceConfig::Json {
                folder: Some(persist.to_owned()),
            }),
            ..Default::default()
        },
    )
    .await
    .context("error creating test session")?;
    Ok(Api::new(
        session,
        None,
        #[cfg(feature = "tracing-subscriber-utils")]
        None,
    ))
}

#[tokio::test(flavor = "multi_thread")]
async fn torrent_rate_limits_persist_across_restart() -> anyhow::Result<()> {
    setup_test_logging();

    let payload = create_default_random_dir_with_torrents(1, 64 * 1024, Some("ratelimit_restart"));
    let torrent = crate::create_torrent(
        payload.path(),
        Default::default(),
        &crate::spawn_utils::BlockingSpawner::new(1),
    )
    .await?
    .as_bytes()?
    .to_vec();
    let root = TempDir::with_prefix("ratelimit_restart_out")?;
    let persist = TempDir::with_prefix("ratelimit_restart_state")?;

    let api = create_session(root.path(), persist.path()).await?;
    let handle = api
        .session()
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder: Some(root.path().join("dl").to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a handle")?;
    let id = TorrentIdOrHash::Id(handle.id());
    let limits = TorrentRateLimits {
        download_bps: NonZeroU32::new(250_000),
        upload_bps: NonZeroU32::new(40_000),
    };
    api.api_set_torrent_rate_limits(id, limits).await?;
    assert_eq!(api.api_torrent_rate_limits(id)?, limits);
    api.session().stop().await;
    drop(api);

    let api = create_session(root.path(), persist.path()).await?;
    assert_eq!(api.api_torrent_rate_limits(id)?, limits, "limits restored");

    // Clearing them is saved too.
    api.api_set_torrent_rate_limits(id, TorrentRateLimits::default())
        .await?;
    api.session().stop().await;
    drop(api);
    let api = create_session(root.path(), persist.path()).await?;
    assert!(api.api_torrent_rate_limits(id)?.is_unlimited());
    api.session().stop().await;
    Ok(())
}
