//! Torrents handed to the app from outside: a `.torrent` file opened in a file
//! manager, a `magnet:` link clicked in a browser, or the file picker. The OS
//! passes these as process arguments (`%U` on Linux), and a second instance
//! forwards its own arguments to the running one.

use std::path::{Path, PathBuf};

use librtbit::AddTorrent;
use tauri::{AppHandle, Manager};
use tracing::{info, warn};

use crate::EngineState;

/// Passed by the autostart entry so the app starts hidden in the tray.
pub const AUTOSTART_ARG: &str = "--autostart";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchTarget {
    Magnet(String),
    TorrentFile(PathBuf),
}

/// Picks torrent targets out of an argv. The first entry is the program name and is skipped.
/// Relative paths resolve against `cwd`, the directory the launching process ran in.
pub fn targets_from_args<S: AsRef<str>>(argv: &[S], cwd: &Path) -> Vec<LaunchTarget> {
    argv.iter()
        .skip(1)
        .filter_map(|arg| classify(arg.as_ref(), cwd))
        .collect()
}

pub fn starts_hidden<S: AsRef<str>>(argv: &[S]) -> bool {
    argv.iter().any(|arg| arg.as_ref() == AUTOSTART_ARG)
}

fn classify(arg: &str, cwd: &Path) -> Option<LaunchTarget> {
    if has_scheme(arg, "magnet:") {
        return Some(LaunchTarget::Magnet(arg.to_owned()));
    }
    let path = if has_scheme(arg, "file:") {
        url::Url::parse(arg).ok()?.to_file_path().ok()?
    } else if arg.contains("://") {
        // Other URLs (http, https, ...) are not launch targets.
        return None;
    } else {
        let path = PathBuf::from(arg);
        if path.is_absolute() {
            path
        } else {
            cwd.join(path)
        }
    };
    let is_torrent = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("torrent"));
    is_torrent.then_some(LaunchTarget::TorrentFile(path))
}

fn has_scheme(arg: &str, scheme: &str) -> bool {
    arg.get(..scheme.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(scheme))
}

/// Adds every target to the engine and returns one message per failure.
pub async fn add_targets(api: &librtbit::Api, targets: Vec<LaunchTarget>) -> Vec<String> {
    let mut failures = Vec::new();
    for target in targets {
        let outcome = match &target {
            LaunchTarget::Magnet(link) => api
                .api_add_torrent(AddTorrent::from_url(link.as_str()), None)
                .await
                .map(|_| ())
                .map_err(|e| format!("{e:#}")),
            LaunchTarget::TorrentFile(path) => match std::fs::read(path) {
                Ok(bytes) => api
                    .api_add_torrent(AddTorrent::from_bytes(bytes), None)
                    .await
                    .map(|_| ())
                    .map_err(|e| format!("{e:#}")),
                Err(e) => Err(format!("could not read {}: {e}", path.display())),
            },
        };
        match outcome {
            Ok(()) => info!(?target, "added torrent from launch request"),
            Err(e) => {
                warn!(?target, "could not add torrent from launch request: {e}");
                failures.push(e);
            }
        }
    }
    failures
}

/// Adds the targets on the engine if it is running, and tells the user about any that fail.
pub fn spawn_add(app: AppHandle, targets: Vec<LaunchTarget>) {
    if targets.is_empty() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let Some(api) = app
            .try_state::<EngineState>()
            .map(|state| state.api.clone())
        else {
            warn!(
                "engine is not running yet; ignoring {} launch target(s)",
                targets.len()
            );
            return;
        };
        for failure in add_targets(&api, targets).await {
            crate::send_notification(&app, "Could not add torrent", &failure);
        }
    });
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{LaunchTarget, starts_hidden, targets_from_args};

    const CWD: &str = "/home/user/Downloads";

    #[test]
    fn program_name_is_never_a_target() {
        let argv = ["/usr/bin/rtbit-desktop.torrent"];
        assert!(targets_from_args(&argv, Path::new(CWD)).is_empty());
    }

    #[test]
    fn magnet_links_are_kept_verbatim_in_any_case() {
        let link = "magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862";
        let argv = ["rtbit-desktop", link, "MAGNET:?xt=urn:btih:abc"];
        assert_eq!(
            targets_from_args(&argv, Path::new(CWD)),
            vec![
                LaunchTarget::Magnet(link.to_owned()),
                LaunchTarget::Magnet("MAGNET:?xt=urn:btih:abc".to_owned()),
            ]
        );
    }

    #[test]
    fn absolute_torrent_paths_are_used_as_given() {
        let argv = ["rtbit-desktop", "/srv/torrents/ubuntu.torrent"];
        assert_eq!(
            targets_from_args(&argv, Path::new(CWD)),
            vec![LaunchTarget::TorrentFile(
                "/srv/torrents/ubuntu.torrent".into()
            )]
        );
    }

    #[test]
    fn relative_torrent_paths_resolve_against_the_launch_directory() {
        let argv = ["rtbit-desktop", "--", "ubuntu.TORRENT"];
        assert_eq!(
            targets_from_args(&argv, Path::new(CWD)),
            vec![LaunchTarget::TorrentFile(
                Path::new(CWD).join("ubuntu.TORRENT")
            )]
        );
    }

    #[test]
    fn file_uris_from_percent_u_are_decoded_to_paths() {
        let argv = ["rtbit-desktop", "file:///home/user/My%20Shows/a.torrent"];
        assert_eq!(
            targets_from_args(&argv, Path::new(CWD)),
            vec![LaunchTarget::TorrentFile(
                "/home/user/My Shows/a.torrent".into()
            )]
        );
    }

    #[test]
    fn non_torrent_arguments_are_ignored() {
        let argv = [
            "rtbit-desktop",
            "--autostart",
            "notes.txt",
            "https://example.com/a.torrent",
            "file:///tmp/readme.md",
            "magnetic:north",
        ];
        assert!(targets_from_args(&argv, Path::new(CWD)).is_empty());
    }

    #[test]
    fn autostart_flag_starts_hidden() {
        assert!(starts_hidden(&["rtbit-desktop", "--autostart"]));
        assert!(!starts_hidden(&["rtbit-desktop", "/x/a.torrent"]));
    }
}
