use std::{collections::HashMap, path::Path};

use anyhow::{Context, bail};
use librtbit_core::torrent_metainfo::torrent_from_bytes;
use serde::{Deserialize, Serialize};

use super::SerializedTorrent;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MissingFileStatus {
    Exists,
    WrongSize { actual_size: u64 },
    Absent,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MissingTorrentDiagnosis {
    PathProblem,
    MissingPayload,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MissingFileReport {
    pub path: std::path::PathBuf,
    pub expected_size: u64,
    pub parent_dir_exists: bool,
    pub status: MissingFileStatus,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MissingTorrentReport {
    pub id: usize,
    pub info_hash: String,
    pub name: String,
    pub save_path: std::path::PathBuf,
    pub diagnosis: MissingTorrentDiagnosis,
    pub files: Vec<MissingFileReport>,
}

#[derive(Deserialize)]
struct SessionDatabase {
    torrents: HashMap<usize, SerializedTorrent>,
}

pub async fn report_missing_files(
    persistence_folder: &Path,
) -> anyhow::Result<Vec<MissingTorrentReport>> {
    let session_file = persistence_folder.join("session.json");
    let bytes = tokio::fs::read(&session_file)
        .await
        .with_context(|| format!("error reading session state {}", session_file.display()))?;
    let database: SessionDatabase = serde_json::from_slice(&bytes)
        .with_context(|| format!("error parsing session state {}", session_file.display()))?;

    let mut torrents = database.torrents.into_iter().collect::<Vec<_>>();
    torrents.sort_by_key(|(id, _)| *id);
    let mut reports = Vec::new();
    for (id, torrent) in torrents {
        let info_hash = torrent.info_hash.as_string();
        let metainfo_file = persistence_folder.join(format!("{info_hash}.torrent"));
        let metainfo_bytes = tokio::fs::read(&metainfo_file)
            .await
            .with_context(|| format!("error reading metainfo {}", metainfo_file.display()))?;
        let metainfo = torrent_from_bytes(&metainfo_bytes)
            .with_context(|| format!("error parsing metainfo {}", metainfo_file.display()))?;
        let metainfo = metainfo
            .info
            .data
            .validate()
            .context("invalid torrent metainfo")?;
        let name = metainfo
            .name()
            .unwrap_or_else(|| "<unnamed>".into())
            .into_owned();
        let mut files = Vec::new();
        let mut has_missing = false;
        let mut has_missing_parent = false;

        for (file_id, details) in metainfo.iter_file_details().enumerate() {
            if torrent
                .only_files
                .as_ref()
                .is_some_and(|selected| !selected.contains(&file_id))
            {
                continue;
            }
            let relative_path = torrent
                .file_renames
                .get(&file_id)
                .cloned()
                .unwrap_or_else(|| details.filename.to_pathbuf());
            if relative_path.is_absolute() {
                bail!("saved file path is absolute: {}", relative_path.display());
            }
            let path = torrent.output_folder.join(relative_path);
            let parent_dir_exists = path.parent().is_some_and(Path::is_dir);
            let status = match tokio::fs::metadata(&path).await {
                Ok(metadata) if metadata.is_file() && metadata.len() == details.len => {
                    MissingFileStatus::Exists
                }
                Ok(metadata) if metadata.is_file() => MissingFileStatus::WrongSize {
                    actual_size: metadata.len(),
                },
                Ok(_) => MissingFileStatus::WrongSize { actual_size: 0 },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    MissingFileStatus::Absent
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("error inspecting {}", path.display()));
                }
            };
            if status != MissingFileStatus::Exists {
                has_missing = true;
                has_missing_parent |= !parent_dir_exists;
            }
            files.push(MissingFileReport {
                path,
                expected_size: details.len,
                parent_dir_exists,
                status,
            });
        }

        if has_missing {
            reports.push(MissingTorrentReport {
                id,
                info_hash,
                name,
                save_path: torrent.output_folder,
                diagnosis: if has_missing_parent {
                    MissingTorrentDiagnosis::PathProblem
                } else {
                    MissingTorrentDiagnosis::MissingPayload
                },
                files,
            });
        }
    }
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CreateTorrentOptions, create_torrent, spawn_utils::BlockingSpawner};

    #[tokio::test]
    async fn reports_file_statuses_and_path_problem() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        for (name, size) in [
            ("exists", 4),
            ("wrong", 5),
            ("absent", 6),
            ("nested/missing", 7),
        ] {
            let path = source.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, vec![b'x'; size]).unwrap();
        }
        let torrent = create_torrent(
            &source,
            CreateTorrentOptions::default(),
            &BlockingSpawner::new(1),
        )
        .await
        .unwrap()
        .as_bytes()
        .unwrap()
        .to_vec();
        let metadata = torrent_from_bytes(&torrent).unwrap();
        let info_hash = metadata.info_hash.as_string();
        let persistence = temp.path().join("state");
        let payload = temp.path().join("payload");
        std::fs::create_dir_all(&persistence).unwrap();
        std::fs::create_dir_all(&payload).unwrap();
        std::fs::write(payload.join("exists"), b"xxxx").unwrap();
        std::fs::write(payload.join("wrong"), b"x").unwrap();
        std::fs::write(persistence.join(format!("{info_hash}.torrent")), &torrent).unwrap();
        let session = serde_json::json!({
            "torrents": {
                "1": {
                    "info_hash": info_hash,
                    "trackers": [],
                    "output_folder": payload,
                    "only_files": null,
                    "is_paused": true
                }
            }
        });
        std::fs::write(
            persistence.join("session.json"),
            serde_json::to_vec(&session).unwrap(),
        )
        .unwrap();

        let reports = report_missing_files(&persistence).await.unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].diagnosis, MissingTorrentDiagnosis::PathProblem);
        assert_eq!(reports[0].files.len(), 4);
        let status_for = |name: &str| {
            reports[0]
                .files
                .iter()
                .find(|file| file.path.ends_with(name))
                .unwrap()
        };
        assert_eq!(status_for("absent").status, MissingFileStatus::Absent);
        assert_eq!(status_for("exists").status, MissingFileStatus::Exists);
        assert!(matches!(
            status_for("wrong").status,
            MissingFileStatus::WrongSize { actual_size: 1 }
        ));
        assert_eq!(status_for("missing").status, MissingFileStatus::Absent);
        assert!(!status_for("missing").parent_dir_exists);
    }
}
