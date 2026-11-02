use std::{collections::HashSet, path::Path};

use anyhow::{Context, bail};
use buffers::ByteBufOwned;
use futures::{Stream, stream::BoxStream};
use librtbit_core::torrent_metainfo::ValidatedTorrentMetaV1Info;
use tracing::{debug, info, warn};

use crate::{merge_streams::merge_streams, storage::TorrentStorage, type_aliases::FileInfos};

use super::types::ParsedTorrentFile;
use super::types::torrent_from_bytes;

pub(super) async fn torrent_from_url(
    reqwest_client: &reqwest::Client,
    url: &str,
) -> anyhow::Result<ParsedTorrentFile> {
    if url.starts_with("file://") {
        let parsed =
            url::Url::parse(url).map_err(|e| anyhow::anyhow!("invalid file URL {url:?}: {e}"))?;
        let path = parsed.to_file_path().map_err(|_| {
            anyhow::anyhow!("file URL cannot be converted to a local path: {url:?}")
        })?;
        let bytes = tokio::fs::read(&path)
            .await
            .with_context(|| format!("error reading torrent metadata from {}", path.display()))?;
        return torrent_from_bytes(bytes.into()).context("error decoding torrent");
    }

    let _ = reqwest_client;
    let b = crate::safe_fetch::fetch_bytes(url, &crate::safe_fetch::FetchLimits::default())
        .await
        .with_context(|| format!("error downloading torrent metadata from {url}"))?;
    torrent_from_bytes(b.into()).context("error decoding torrent")
}

pub(super) fn compute_only_files_regex<ByteBuf: AsRef<[u8]>>(
    torrent: &ValidatedTorrentMetaV1Info<ByteBuf>,
    filename_re: &str,
) -> anyhow::Result<Vec<usize>> {
    let filename_re = regex::Regex::new(filename_re).context("filename regex is incorrect")?;
    let mut only_files = Vec::new();
    for (idx, fd) in torrent.iter_file_details().enumerate() {
        let full_path = fd.filename.to_pathbuf();
        let Some(path_str) = full_path.to_str() else {
            continue;
        };
        if filename_re.is_match(path_str) {
            only_files.push(idx);
        }
    }
    if only_files.is_empty() {
        bail!("none of the filenames match the given regex")
    }
    Ok(only_files)
}

pub(super) fn compute_only_files(
    info: &ValidatedTorrentMetaV1Info<ByteBufOwned>,
    only_files: Option<Vec<usize>>,
    only_files_regex: Option<String>,
    list_only: bool,
) -> anyhow::Result<Option<Vec<usize>>> {
    match (only_files, only_files_regex) {
        (Some(_), Some(_)) => {
            bail!("only_files and only_files_regex are mutually exclusive");
        }
        (Some(only_files), None) => {
            let total_files = info.iter_file_lengths().count();
            for id in only_files.iter().copied() {
                if id >= total_files {
                    bail!("file id {id} is out of range");
                }
            }
            Ok(Some(only_files))
        }
        (None, Some(filename_re)) => {
            let only_files = compute_only_files_regex(info, &filename_re)?;
            for (idx, fd) in info.iter_file_details().enumerate() {
                if !only_files.contains(&idx) {
                    continue;
                }
                if !list_only {
                    info!(filename=?fd.filename, "will download");
                }
            }
            Ok(Some(only_files))
        }
        (None, None) => Ok(None),
    }
}

pub(super) fn merge_two_optional_streams<T>(
    s1: Option<impl Stream<Item = T> + Unpin + Send + 'static>,
    s2: Option<impl Stream<Item = T> + Unpin + Send + 'static>,
) -> Option<BoxStream<'static, T>> {
    match (s1, s2) {
        (Some(s1), None) => Some(Box::pin(s1)),
        (None, Some(s2)) => Some(Box::pin(s2)),
        (Some(s1), Some(s2)) => Some(Box::pin(merge_streams(s1, s2))),
        (None, None) => None,
    }
}

pub(crate) fn remove_files_and_dirs(infos: &FileInfos, files: &dyn TorrentStorage) {
    let mut all_dirs = HashSet::new();
    for (id, fi) in infos.iter().enumerate() {
        if fi.attrs.padding {
            continue;
        }
        let mut fname = &*fi.relative_filename;
        if let Err(e) = files.remove_file(id, fname) {
            warn!(?fi.relative_filename, error=?e, "could not delete file");
        } else {
            debug!(?fi.relative_filename, "deleted the file")
        }
        while let Some(parent) = fname.parent() {
            if parent != Path::new("") {
                all_dirs.insert(parent);
            }
            fname = parent;
        }
    }

    let all_dirs = {
        let mut v = all_dirs.into_iter().collect::<Vec<_>>();
        v.sort_unstable_by_key(|p| std::cmp::Reverse(p.as_os_str().len()));
        v
    };
    for dir in all_dirs {
        if let Err(e) = files.remove_directory_if_empty(dir) {
            warn!("error removing {dir:?}: {e:#}");
        } else {
            debug!("removed {dir:?}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_torrent_file::create_torrent;
    use crate::spawn_utils::BlockingSpawner;
    use crate::tests::test_util;
    use futures::StreamExt;

    /// Helper to create a multi-file torrent and return the parsed validated info.
    async fn make_multi_file_torrent_info() -> ValidatedTorrentMetaV1Info<ByteBufOwned> {
        let dir = tempfile::tempdir().unwrap();
        test_util::create_new_file_with_random_content(&dir.path().join("movie.mp4"), 5000);
        test_util::create_new_file_with_random_content(&dir.path().join("subtitle.srt"), 500);
        test_util::create_new_file_with_random_content(&dir.path().join("readme.txt"), 200);

        let result = create_torrent(dir.path(), Default::default(), &BlockingSpawner::new(1))
            .await
            .unwrap();

        let bytes = result.as_bytes().unwrap();
        let parsed = torrent_from_bytes(bytes).unwrap();
        parsed.meta.info.data.validate().unwrap()
    }

    #[tokio::test]
    async fn torrent_from_file_url_reads_local_torrent() {
        let dir = tempfile::tempdir().unwrap();
        let torrent_path = dir.path().join("ubuntu.torrent");
        tokio::fs::copy(
            "resources/ubuntu-21.04-desktop-amd64.iso.torrent",
            &torrent_path,
        )
        .await
        .unwrap();
        let url = url::Url::from_file_path(&torrent_path).unwrap().to_string();
        let client = reqwest::Client::new();

        let parsed = super::torrent_from_url(&client, &url).await.unwrap();

        assert!(!parsed.torrent_bytes.is_empty());
    }

    #[tokio::test]
    async fn test_compute_only_files_regex() {
        let info = make_multi_file_torrent_info().await;
        // Match only .mp4 files
        let result = compute_only_files_regex(&info, r"\.mp4$").unwrap();
        assert_eq!(result.len(), 1);
    }

    #[tokio::test]
    async fn test_compute_only_files_regex_no_match() {
        let info = make_multi_file_torrent_info().await;
        let result = compute_only_files_regex(&info, r"\.nonexistent$");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_compute_only_files_by_index() {
        let info = make_multi_file_torrent_info().await;
        let result = compute_only_files(&info, Some(vec![0, 1]), None, false).unwrap();
        assert_eq!(result, Some(vec![0, 1]));
    }

    #[tokio::test]
    async fn test_compute_only_files_invalid_index() {
        let info = make_multi_file_torrent_info().await;
        let result = compute_only_files(&info, Some(vec![999]), None, false);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_compute_only_files_mutually_exclusive() {
        let info = make_multi_file_torrent_info().await;
        let result = compute_only_files(&info, Some(vec![0]), Some(r"\.mp4$".to_string()), false);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("mutually exclusive"));
    }

    #[tokio::test]
    async fn test_compute_only_files_none() {
        let info = make_multi_file_torrent_info().await;
        let result = compute_only_files(&info, None, None, false).unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_merge_two_optional_streams_both_none() {
        let none1: Option<futures::stream::Iter<std::vec::IntoIter<i32>>> = None;
        let none2: Option<futures::stream::Iter<std::vec::IntoIter<i32>>> = None;
        let result = merge_two_optional_streams(none1, none2);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_merge_two_optional_streams_first_only() {
        let s1 = futures::stream::iter(vec![1, 2, 3]);
        let none2: Option<futures::stream::Iter<std::vec::IntoIter<i32>>> = None;
        let result = merge_two_optional_streams(Some(s1), none2);
        assert!(result.is_some());
        let items: Vec<_> = result.unwrap().collect().await;
        assert_eq!(items, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn test_merge_two_optional_streams_second_only() {
        let none1: Option<futures::stream::Iter<std::vec::IntoIter<i32>>> = None;
        let s2 = futures::stream::iter(vec![4, 5, 6]);
        let result = merge_two_optional_streams(none1, Some(s2));
        assert!(result.is_some());
        let items: Vec<_> = result.unwrap().collect().await;
        assert_eq!(items, vec![4, 5, 6]);
    }

    #[tokio::test]
    async fn test_merge_two_optional_streams_both() {
        let s1 = futures::stream::iter(vec![1, 2]);
        let s2 = futures::stream::iter(vec![3, 4]);
        let result = merge_two_optional_streams(Some(s1), Some(s2));
        assert!(result.is_some());
        let mut items: Vec<_> = result.unwrap().collect().await;
        items.sort();
        assert_eq!(items, vec![1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn test_torrent_from_bytes_valid() {
        let dir = tempfile::tempdir().unwrap();
        test_util::create_new_file_with_random_content(&dir.path().join("test.bin"), 1000);

        let result = create_torrent(dir.path(), Default::default(), &BlockingSpawner::new(1))
            .await
            .unwrap();

        let bytes = result.as_bytes().unwrap();
        let parsed = torrent_from_bytes(bytes);
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_torrent_from_bytes_invalid() {
        let invalid = bytes::Bytes::from_static(b"this is not a valid torrent file");
        let result = torrent_from_bytes(invalid);
        assert!(result.is_err());
    }
}
