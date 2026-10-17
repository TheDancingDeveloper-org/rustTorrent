//! Actions run when a torrent finishes downloading: an operator-supplied shell
//! command and/or a webhook that receives a JSON POST.
//!
//! Both run detached from the torrent's lifecycle. Failures are logged and never
//! affect the torrent itself.

use std::{path::PathBuf, time::Duration};

use serde::Serialize;
use tracing::{debug, info, warn};

use super::{Session, TorrentId};

/// How long a webhook POST may take before it is abandoned.
const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(30);

/// Configuration for the on-complete hook. Both fields are optional; the hook
/// is inert when neither is set.
#[derive(Debug, Clone, Default)]
pub struct OnCompleteHook {
    /// Shell command run with `sh -c`. It receives the `RTBIT_*` variables from
    /// [`CompletedTorrent::env`] in its environment.
    pub command: Option<String>,
    /// URL that receives a JSON body (see [`CompletedTorrent`]) via POST.
    pub webhook_url: Option<String>,
}

impl OnCompleteHook {
    pub fn is_enabled(&self) -> bool {
        self.command.is_some() || self.webhook_url.is_some()
    }
}

impl Session {
    /// Replace the on-complete hook. Applies to completions after the call.
    pub fn set_on_complete_hook(&self, hook: OnCompleteHook) {
        *self.on_complete.write() = hook;
    }

    /// The current on-complete hook.
    pub fn on_complete_hook(&self) -> OnCompleteHook {
        self.on_complete.read().clone()
    }
}

/// The event describing one finished torrent. Serialized as the webhook body.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CompletedTorrent {
    pub event: &'static str,
    pub id: TorrentId,
    pub info_hash: String,
    pub name: Option<String>,
    pub output_folder: PathBuf,
    /// Unix timestamp (seconds) of the completion.
    pub completed_at: u64,
}

impl CompletedTorrent {
    pub const EVENT: &'static str = "torrent_completed";

    pub fn new(
        id: TorrentId,
        info_hash: String,
        name: Option<String>,
        output_folder: PathBuf,
        completed_at: u64,
    ) -> Self {
        Self {
            event: Self::EVENT,
            id,
            info_hash,
            name,
            output_folder,
            completed_at,
        }
    }

    /// Environment passed to the shell command.
    pub fn env(&self) -> Vec<(&'static str, String)> {
        vec![
            ("RTBIT_EVENT", self.event.to_owned()),
            ("RTBIT_TORRENT_ID", self.id.to_string()),
            ("RTBIT_INFO_HASH", self.info_hash.clone()),
            ("RTBIT_TORRENT_NAME", self.name.clone().unwrap_or_default()),
            (
                "RTBIT_OUTPUT_FOLDER",
                self.output_folder.to_string_lossy().into_owned(),
            ),
            ("RTBIT_COMPLETED_AT", self.completed_at.to_string()),
        ]
    }
}

/// Run every configured action for one completed torrent. Errors are logged.
pub(crate) async fn run(hook: OnCompleteHook, client: reqwest::Client, event: CompletedTorrent) {
    if let Some(command) = hook.command {
        let env = event.env();
        let id = event.id;
        let result = tokio::task::spawn_blocking(move || {
            std::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
                .status()
        })
        .await;
        match result {
            Ok(Ok(status)) if status.success() => {
                info!(id, "on-complete command finished");
            }
            Ok(Ok(status)) => warn!(id, ?status, "on-complete command exited unsuccessfully"),
            Ok(Err(e)) => warn!(id, "failed to start on-complete command: {e:#}"),
            Err(e) => warn!(id, "on-complete command task failed: {e:#}"),
        }
    }

    if let Some(url) = hook.webhook_url {
        match client
            .post(&url)
            .timeout(WEBHOOK_TIMEOUT)
            .json(&event)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                debug!(id = event.id, "on-complete webhook delivered");
            }
            Ok(resp) => {
                warn!(id = event.id, status = %resp.status(), "on-complete webhook rejected");
            }
            Err(e) => warn!(id = event.id, "on-complete webhook failed: {e:#}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: Option<&str>) -> CompletedTorrent {
        CompletedTorrent::new(
            7,
            "aabbcc".to_owned(),
            name.map(str::to_owned),
            PathBuf::from("/downloads/movies"),
            1_700_000_000,
        )
    }

    #[test]
    fn disabled_by_default() {
        assert!(!OnCompleteHook::default().is_enabled());
        let command_only = OnCompleteHook {
            command: Some("true".into()),
            ..Default::default()
        };
        assert!(command_only.is_enabled());
    }

    #[test]
    fn env_carries_torrent_identity() {
        let env: std::collections::HashMap<_, _> =
            sample(Some("Movie")).env().into_iter().collect();
        assert_eq!(env["RTBIT_EVENT"], "torrent_completed");
        assert_eq!(env["RTBIT_TORRENT_ID"], "7");
        assert_eq!(env["RTBIT_INFO_HASH"], "aabbcc");
        assert_eq!(env["RTBIT_TORRENT_NAME"], "Movie");
        assert_eq!(env["RTBIT_OUTPUT_FOLDER"], "/downloads/movies");
        assert_eq!(env["RTBIT_COMPLETED_AT"], "1700000000");
    }

    #[test]
    fn env_name_is_empty_when_unknown() {
        let env: std::collections::HashMap<_, _> = sample(None).env().into_iter().collect();
        assert_eq!(env["RTBIT_TORRENT_NAME"], "");
    }

    #[test]
    fn webhook_body_shape() {
        let json = serde_json::to_value(sample(Some("Movie"))).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "event": "torrent_completed",
                "id": 7,
                "info_hash": "aabbcc",
                "name": "Movie",
                "output_folder": "/downloads/movies",
                "completed_at": 1_700_000_000u64,
            })
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn command_receives_environment() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.txt");
        let hook = OnCompleteHook {
            command: Some(format!(
                "printf '%s|%s' \"$RTBIT_INFO_HASH\" \"$RTBIT_TORRENT_NAME\" > '{}'",
                out.display()
            )),
            webhook_url: None,
        };
        run(hook, reqwest::Client::new(), sample(Some("Movie"))).await;
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "aabbcc|Movie");
    }
}
