use std::{
    fs::OpenOptions,
    io::IoSlice,
    path::{Path, PathBuf},
};

use anyhow::Context;
use parking_lot::RwLock;
use tracing::warn;

use crate::{
    storage::{StorageFactoryExt, filesystem::opened_file::OurFileExt},
    torrent_state::{ManagedTorrentShared, TorrentMetadata},
};

use crate::storage::{StorageFactory, TorrentStorage};

use super::opened_file::OpenedFile;

/// Name an incomplete file is written under when `part_files` is on: the final
/// name with `.part` appended.
pub(crate) fn part_path(final_path: &Path) -> PathBuf {
    let mut name = final_path.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

#[derive(Default, Clone, Copy)]
pub struct FilesystemStorageFactory {
    read_only: bool,
}

impl FilesystemStorageFactory {
    /// Create storage that can verify and seed existing payloads without
    /// creating, resizing, writing, or deleting payload files.
    pub const fn read_only() -> Self {
        Self { read_only: true }
    }
}

impl StorageFactory for FilesystemStorageFactory {
    type Storage = FilesystemStorage;

    fn create(
        &self,
        shared: &ManagedTorrentShared,
        _metadata: &TorrentMetadata,
    ) -> anyhow::Result<FilesystemStorage> {
        Ok(FilesystemStorage {
            output_folder: RwLock::new(shared.output_folder_override.read().clone()),
            opened_files: Default::default(),
            read_only: self.read_only,
        })
    }

    fn clone_box(&self) -> crate::storage::BoxStorageFactory {
        self.boxed()
    }
}

pub struct FilesystemStorage {
    /// Storage root. Mutable so a whole-torrent relocation (`move_root`) can
    /// re-anchor the storage; only cold paths (init/remove/rename/move) read it.
    pub(super) output_folder: RwLock<PathBuf>,
    pub(super) opened_files: Vec<OpenedFile>,
    read_only: bool,
}

impl FilesystemStorage {
    fn require_writable(&self) -> anyhow::Result<()> {
        if self.read_only {
            anyhow::bail!("filesystem storage is read-only")
        }
        Ok(())
    }

    pub(super) fn take_fs(&self) -> anyhow::Result<Self> {
        Ok(Self {
            opened_files: self
                .opened_files
                .iter()
                .map(|f| f.take_clone())
                .collect::<anyhow::Result<Vec<_>>>()?,
            output_folder: RwLock::new(self.output_folder.read().clone()),
            read_only: self.read_only,
        })
    }
}

impl TorrentStorage for FilesystemStorage {
    fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn pread_exact(&self, file_id: usize, offset: u64, buf: &mut [u8]) -> anyhow::Result<()> {
        self.opened_files
            .get(file_id)
            .context("no such file")?
            .lock_read()?
            .pread_exact(offset, buf)
    }

    fn pwrite_all(&self, file_id: usize, offset: u64, buf: &[u8]) -> anyhow::Result<()> {
        self.require_writable()?;
        let of = self.opened_files.get(file_id).context("no such file")?;
        #[cfg(windows)]
        return of.try_mark_sparse()?.pwrite_all(offset, buf);
        #[cfg(not(windows))]
        return of.lock_read()?.pwrite_all(offset, buf);
    }

    fn pwrite_all_vectored(
        &self,
        file_id: usize,
        offset: u64,
        bufs: [IoSlice<'_>; 2],
    ) -> anyhow::Result<usize> {
        self.require_writable()?;
        let of = self.opened_files.get(file_id).context("no such file")?;
        #[cfg(windows)]
        return of.try_mark_sparse()?.pwrite_all_vectored(offset, bufs);
        #[cfg(not(windows))]
        return of.lock_read()?.pwrite_all_vectored(offset, bufs);
    }

    fn remove_file(&self, _file_id: usize, filename: &Path) -> anyhow::Result<()> {
        self.require_writable()?;
        let full = self.output_folder.read().join(filename);
        // An incomplete file may still be under its `.part` name.
        let target = if !full.exists() && part_path(&full).exists() {
            part_path(&full)
        } else {
            full
        };
        Ok(std::fs::remove_file(target)?)
    }

    fn rename_file(&self, file_id: usize, new_relative: &Path) -> anyhow::Result<()> {
        self.require_writable()?;
        let of = self.opened_files.get(file_id).context("no such file")?;
        let new_full = self.output_folder.read().join(new_relative);
        of.rename_to(&new_full)
    }

    fn move_root(&self, new_root: &Path) -> anyhow::Result<()> {
        self.require_writable()?;
        let old_root = self.output_folder.read().clone();
        if old_root == new_root {
            return Ok(());
        }
        // Move each file to the same relative location under the new root. On
        // any failure (including a cross-filesystem EXDEV rename), roll the
        // already-moved files back before returning.
        for (moved, of) in self.opened_files.iter().enumerate() {
            if let Err(error) = of.rebase(&old_root, new_root) {
                for prev in self.opened_files[..moved].iter().rev() {
                    if let Err(rollback_error) = prev.rebase(new_root, &old_root) {
                        warn!(
                            %rollback_error,
                            "error rolling back a file after a failed relocation; \
                             storage may be left inconsistent for this torrent"
                        );
                    }
                }
                return Err(error).context("error relocating torrent; rolled back");
            }
        }
        *self.output_folder.write() = new_root.to_owned();
        Ok(())
    }

    fn ensure_file_length(&self, file_id: usize, len: u64) -> anyhow::Result<()> {
        let f = &self.opened_files.get(file_id).context("no such file")?;
        if self.read_only {
            let actual = f.lock_read()?.metadata()?.len();
            if actual != len {
                anyhow::bail!(
                    "filesystem storage is read-only and file length is {actual}, expected {len}"
                )
            }
            return Ok(());
        }
        #[cfg(windows)]
        f.try_mark_sparse()?;
        Ok(f.lock_read()?.set_len(len)?)
    }

    fn take(&self) -> anyhow::Result<Box<dyn TorrentStorage>> {
        Ok(Box::new(Self {
            opened_files: self
                .opened_files
                .iter()
                .map(|f| f.take_clone())
                .collect::<anyhow::Result<Vec<_>>>()?,
            output_folder: RwLock::new(self.output_folder.read().clone()),
            read_only: self.read_only,
        }))
    }

    fn remove_directory_if_empty(&self, path: &Path) -> anyhow::Result<()> {
        self.require_writable()?;
        let path = self.output_folder.read().join(path);
        if !path.is_dir() {
            anyhow::bail!("cannot remove dir: {path:?} is not a directory")
        }
        // Attempt removal directly to avoid TOCTOU race between empty check and remove.
        // std::fs::remove_dir only removes empty directories; if non-empty it errors.
        match std::fs::remove_dir(&path) {
            Ok(()) => Ok(()),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::DirectoryNotEmpty) => {
                warn!("did not remove {path:?} as it was not empty");
                Ok(())
            }
            Err(e) => Err(e).with_context(|| format!("error removing {path:?}")),
        }
    }

    fn init(
        &mut self,
        shared: &ManagedTorrentShared,
        metadata: &TorrentMetadata,
    ) -> anyhow::Result<()> {
        let mut files = Vec::<OpenedFile>::new();
        let output_folder = self.output_folder.read().clone();
        for file_details in metadata.file_infos.iter() {
            let mut full_path = output_folder.clone();
            let relative_path = &file_details.relative_filename;
            full_path.push(relative_path);

            if file_details.attrs.padding {
                files.push(OpenedFile::new_dummy());
                continue;
            };
            if self.read_only {
                match OpenOptions::new().read(true).open(&full_path) {
                    Ok(f) => files.push(OpenedFile::new(full_path, f)),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        files.push(OpenedFile::new_dummy());
                    }
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!("error opening {full_path:?} in read-only mode")
                        });
                    }
                }
                continue;
            }

            // While a file is incomplete it lives under `<name>.part`, unless the
            // final name already exists (a finished file, or one the user placed).
            // An existing `.part` is adopted even when `part_files` is off, so a
            // partial download is never orphaned and redownloaded.
            if !full_path.exists() {
                let part = part_path(&full_path);
                if shared.options.part_files || part.exists() {
                    full_path = part;
                }
            }

            std::fs::create_dir_all(full_path.parent().context("bug: no parent")?)?;
            let f = if shared.options.allow_overwrite {
                OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .read(true)
                    .write(true)
                    .open(&full_path)
                    .with_context(|| format!("error opening {full_path:?} in read/write mode"))?
            } else {
                // create_new does not seem to work with read(true), so calling this twice.
                OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&full_path)
                    .with_context(|| {
                        format!(
                            "error creating a new file (because allow_overwrite = false) {:?}",
                            full_path
                        )
                    })?;
                OpenOptions::new().read(true).write(true).open(&full_path)?
            };
            files.push(OpenedFile::new(full_path.clone(), f));
        }

        self.opened_files = files;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::TorrentStorage;

    /// Helper to create a FilesystemStorage with `n` real files in a temp directory.
    /// Returns (storage, tempdir) -- tempdir must be kept alive for the duration of the test.
    fn make_test_storage(n: usize) -> (FilesystemStorage, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut opened_files = Vec::new();
        for i in 0..n {
            let path = dir.path().join(format!("file_{i}.dat"));
            let f = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            opened_files.push(OpenedFile::new(path, f));
        }
        let storage = FilesystemStorage {
            output_folder: RwLock::new(dir.path().to_owned()),
            opened_files,
            read_only: false,
        };
        (storage, dir)
    }

    #[test]
    fn test_filesystem_storage_write_and_read() {
        let (storage, _dir) = make_test_storage(1);

        // Ensure the file is long enough.
        storage.ensure_file_length(0, 1024).unwrap();

        let write_data = b"hello, world! this is test data.";
        storage.pwrite_all(0, 0, write_data).unwrap();

        let mut read_buf = vec![0u8; write_data.len()];
        storage.pread_exact(0, 0, &mut read_buf).unwrap();

        assert_eq!(&read_buf, write_data);
    }

    #[test]
    fn test_filesystem_storage_write_at_offset() {
        let (storage, _dir) = make_test_storage(1);

        storage.ensure_file_length(0, 4096).unwrap();

        let write_data = b"offset_data";
        let offset = 2048;
        storage.pwrite_all(0, offset, write_data).unwrap();

        let mut read_buf = vec![0u8; write_data.len()];
        storage.pread_exact(0, offset, &mut read_buf).unwrap();

        assert_eq!(&read_buf, write_data);
    }

    #[test]
    fn test_filesystem_storage_piece_hash_verification() {
        use sha1w::ISha1;

        let (storage, _dir) = make_test_storage(1);

        storage.ensure_file_length(0, 16384).unwrap();

        // Write a "piece" of known data.
        #[allow(clippy::cast_possible_truncation)]
        let piece_data: Vec<u8> = (0..16384u32).map(|i| (i % 256) as u8).collect();
        storage.pwrite_all(0, 0, &piece_data).unwrap();

        // Compute expected hash.
        let mut hasher = sha1w::Sha1::new();
        hasher.update(&piece_data);
        let expected_hash = hasher.finish();

        // Read back and compute hash.
        let mut read_buf = vec![0u8; 16384];
        storage.pread_exact(0, 0, &mut read_buf).unwrap();

        let mut hasher2 = sha1w::Sha1::new();
        hasher2.update(&read_buf);
        let actual_hash = hasher2.finish();

        assert_eq!(expected_hash, actual_hash);
    }

    #[test]
    fn test_storage_handles_sparse_writes() {
        let (storage, _dir) = make_test_storage(1);

        storage.ensure_file_length(0, 8192).unwrap();

        // Write at non-sequential offsets (sparse writes).
        let data_a = b"chunk_a";
        let data_b = b"chunk_b";

        // Write chunk B first (at offset 4096), then chunk A (at offset 0).
        storage.pwrite_all(0, 4096, data_b).unwrap();
        storage.pwrite_all(0, 0, data_a).unwrap();

        // Read both back.
        let mut buf_a = vec![0u8; data_a.len()];
        let mut buf_b = vec![0u8; data_b.len()];
        storage.pread_exact(0, 0, &mut buf_a).unwrap();
        storage.pread_exact(0, 4096, &mut buf_b).unwrap();

        assert_eq!(&buf_a, data_a);
        assert_eq!(&buf_b, data_b);
    }

    #[test]
    fn test_storage_file_boundaries() {
        // Test writing to multiple files (pieces spanning file boundaries).
        let (storage, _dir) = make_test_storage(3);

        for i in 0..3 {
            storage.ensure_file_length(i, 1024).unwrap();
        }

        // Write different data to each file.
        let data = [b"file0_data", b"file1_data", b"file2_data"];
        for (i, d) in data.iter().enumerate() {
            storage.pwrite_all(i, 0, *d).unwrap();
        }

        // Read back from each file and verify.
        for (i, d) in data.iter().enumerate() {
            let mut buf = vec![0u8; d.len()];
            storage.pread_exact(i, 0, &mut buf).unwrap();
            assert_eq!(&buf, *d, "file {i} data mismatch");
        }
    }

    #[test]
    fn test_storage_ensure_file_length() {
        let (storage, dir) = make_test_storage(1);

        // Set to 4096 bytes.
        storage.ensure_file_length(0, 4096).unwrap();
        let meta = std::fs::metadata(dir.path().join("file_0.dat")).unwrap();
        assert_eq!(meta.len(), 4096);

        // Grow to 8192 bytes.
        storage.ensure_file_length(0, 8192).unwrap();
        let meta = std::fs::metadata(dir.path().join("file_0.dat")).unwrap();
        assert_eq!(meta.len(), 8192);
    }

    #[test]
    fn test_storage_remove_file() {
        let (storage, dir) = make_test_storage(2);

        let file0_path = dir.path().join("file_0.dat");
        let file1_path = dir.path().join("file_1.dat");
        assert!(file0_path.exists());
        assert!(file1_path.exists());

        // Remove file 0.
        storage.remove_file(0, Path::new("file_0.dat")).unwrap();
        assert!(!file0_path.exists());
        // File 1 should still exist.
        assert!(file1_path.exists());
    }

    #[test]
    fn test_storage_rename_file_moves_and_keeps_handle_usable() {
        let (storage, dir) = make_test_storage(1);
        storage.ensure_file_length(0, 64).unwrap();
        storage.pwrite_all(0, 0, b"payload").unwrap();

        // Rename into a fresh subdirectory (must be created).
        storage
            .rename_file(0, Path::new("sub/renamed.dat"))
            .unwrap();

        // Old path is gone, new path exists with the data.
        assert!(!dir.path().join("file_0.dat").exists());
        let new_path = dir.path().join("sub/renamed.dat");
        assert!(new_path.exists());
        assert_eq!(&std::fs::read(&new_path).unwrap()[..7], b"payload");

        // The cached handle still reads/writes at the new location.
        let mut buf = vec![0u8; 7];
        storage.pread_exact(0, 0, &mut buf).unwrap();
        assert_eq!(&buf, b"payload");
        storage.pwrite_all(0, 0, b"updated").unwrap();
        assert_eq!(&std::fs::read(&new_path).unwrap()[..7], b"updated");
    }

    #[test]
    fn test_storage_rename_file_refuses_to_clobber_existing_destination() {
        let (storage, dir) = make_test_storage(1);
        storage.ensure_file_length(0, 8).unwrap();
        storage.pwrite_all(0, 0, b"torrent!").unwrap();

        // An unrelated file already sits at the target path.
        let victim = dir.path().join("unrelated.dat");
        std::fs::write(&victim, b"precious").unwrap();

        // The rename must be refused, and neither file may be touched.
        assert!(storage.rename_file(0, Path::new("unrelated.dat")).is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
        assert!(dir.path().join("file_0.dat").exists());
        // The torrent file's handle is still usable at its original location.
        let mut buf = vec![0u8; 8];
        storage.pread_exact(0, 0, &mut buf).unwrap();
        assert_eq!(&buf, b"torrent!");
    }

    #[test]
    fn test_storage_move_root_relocates_all_files_and_keeps_handles() {
        let (storage, dir) = make_test_storage(2);
        storage.ensure_file_length(0, 16).unwrap();
        storage.ensure_file_length(1, 16).unwrap();
        storage.pwrite_all(0, 0, b"aaaa").unwrap();
        storage.pwrite_all(1, 0, b"bbbb").unwrap();

        let new_root = dir.path().join("moved");
        storage.move_root(&new_root).unwrap();

        assert!(!dir.path().join("file_0.dat").exists());
        assert!(new_root.join("file_0.dat").exists());
        assert!(new_root.join("file_1.dat").exists());

        // Handles still read at the new location, and a write lands there.
        let mut buf = vec![0u8; 4];
        storage.pread_exact(0, 0, &mut buf).unwrap();
        assert_eq!(&buf, b"aaaa");
        storage.pwrite_all(1, 0, b"cccc").unwrap();
        assert_eq!(
            &std::fs::read(new_root.join("file_1.dat")).unwrap()[..4],
            b"cccc"
        );

        // The anchor moved: path-based ops now resolve under the new root.
        storage.remove_file(0, Path::new("file_0.dat")).unwrap();
        assert!(!new_root.join("file_0.dat").exists());
    }

    #[test]
    fn test_storage_move_root_rolls_back_on_partial_failure() {
        let (storage, dir) = make_test_storage(2);
        storage.ensure_file_length(0, 8).unwrap();
        storage.ensure_file_length(1, 8).unwrap();
        storage.pwrite_all(0, 0, b"keepme").unwrap();

        let new_root = dir.path().join("dest");
        std::fs::create_dir_all(&new_root).unwrap();
        // Block the second file's destination so the relocation fails midway.
        std::fs::write(new_root.join("file_1.dat"), b"blocker").unwrap();

        assert!(storage.move_root(&new_root).is_err());

        // File 0 was rolled back to the original root and is still usable.
        assert!(dir.path().join("file_0.dat").exists());
        assert!(!new_root.join("file_0.dat").exists());
        let mut buf = vec![0u8; 6];
        storage.pread_exact(0, 0, &mut buf).unwrap();
        assert_eq!(&buf, b"keepme");
        // The unrelated blocker file is untouched.
        assert_eq!(
            std::fs::read(new_root.join("file_1.dat")).unwrap(),
            b"blocker"
        );
    }

    #[test]
    fn test_storage_rename_file_rejected_when_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.dat");
        std::fs::write(&path, b"x").unwrap();
        let file = OpenOptions::new().read(true).open(&path).unwrap();
        let storage = FilesystemStorage {
            output_folder: RwLock::new(dir.path().to_owned()),
            opened_files: vec![OpenedFile::new(path, file)],
            read_only: true,
        };
        assert!(storage.rename_file(0, Path::new("other.dat")).is_err());
    }

    #[test]
    fn test_storage_take() {
        let (storage, _dir) = make_test_storage(1);

        storage.ensure_file_length(0, 1024).unwrap();
        storage.pwrite_all(0, 0, b"before_take").unwrap();

        let taken = storage.take().unwrap();

        // The taken storage should be able to read the same data.
        let mut buf = vec![0u8; 11];
        taken.pread_exact(0, 0, &mut buf).unwrap();
        assert_eq!(&buf, b"before_take");
    }

    #[test]
    fn test_storage_invalid_file_id() {
        let (storage, _dir) = make_test_storage(1);

        // File ID 5 does not exist.
        let result = storage.pread_exact(5, 0, &mut [0u8; 10]);
        assert!(result.is_err());

        let result = storage.pwrite_all(5, 0, &[0u8; 10]);
        assert!(result.is_err());
    }

    #[test]
    fn test_storage_remove_directory_if_empty() {
        let (storage, dir) = make_test_storage(0);

        // Create a subdirectory.
        let sub = dir.path().join("subdir");
        std::fs::create_dir(&sub).unwrap();

        // Remove empty directory should succeed.
        storage
            .remove_directory_if_empty(Path::new("subdir"))
            .unwrap();
        assert!(!sub.exists());
    }

    #[test]
    fn test_storage_remove_directory_not_empty() {
        let (storage, dir) = make_test_storage(0);

        // Create a subdirectory with a file inside.
        let sub = dir.path().join("subdir");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("file.txt"), b"content").unwrap();

        // Removing a non-empty directory should not delete it (just warn).
        storage
            .remove_directory_if_empty(Path::new("subdir"))
            .unwrap();
        assert!(sub.exists());
    }

    #[test]
    fn test_read_only_storage_reads_and_rejects_mutations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.dat");
        let original = b"existing payload";
        std::fs::write(&path, original).unwrap();
        let file = OpenOptions::new().read(true).open(&path).unwrap();
        let storage = FilesystemStorage {
            output_folder: RwLock::new(dir.path().to_owned()),
            opened_files: vec![OpenedFile::new(path.clone(), file)],
            read_only: true,
        };

        let mut buf = vec![0; original.len()];
        storage.pread_exact(0, 0, &mut buf).unwrap();
        assert_eq!(buf, original);
        storage
            .ensure_file_length(0, original.len() as u64)
            .unwrap();

        assert!(storage.pwrite_all(0, 0, b"changed").is_err());
        assert!(
            storage
                .pwrite_all_vectored(0, 0, [IoSlice::new(b"changed"), IoSlice::new(b" again")],)
                .is_err()
        );
        assert!(storage.ensure_file_length(0, 1).is_err());
        assert!(storage.remove_file(0, Path::new("payload.dat")).is_err());

        let empty_dir = dir.path().join("empty");
        std::fs::create_dir(&empty_dir).unwrap();
        assert!(
            storage
                .remove_directory_if_empty(Path::new("empty"))
                .is_err()
        );

        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(empty_dir.exists());

        let taken = storage.take().unwrap();
        assert!(taken.pwrite_all(0, 0, b"changed").is_err());
    }

    #[test]
    fn part_path_appends_suffix() {
        assert_eq!(
            part_path(Path::new("/dl/movie/file.mkv")),
            PathBuf::from("/dl/movie/file.mkv.part")
        );
    }

    #[test]
    fn finalize_renames_part_file_and_keeps_handle_writable() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("file_0.dat.part");
        let f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&part)
            .unwrap();
        let storage = FilesystemStorage {
            output_folder: RwLock::new(dir.path().to_owned()),
            opened_files: vec![OpenedFile::new(part.clone(), f)],
            read_only: false,
        };

        storage.pwrite_all(0, 0, b"hello").unwrap();
        storage
            .rename_file(0, Path::new("file_0.dat"))
            .expect("finalize");

        assert!(!part.exists());
        assert_eq!(
            std::fs::read(dir.path().join("file_0.dat")).unwrap(),
            b"hello"
        );
        // The cached handle follows the rename.
        storage.pwrite_all(0, 5, b"!").unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("file_0.dat")).unwrap(),
            b"hello!"
        );
    }

    #[test]
    fn finalize_is_noop_when_already_final() {
        let (storage, dir) = make_test_storage(1);
        storage
            .rename_file(0, Path::new("file_0.dat"))
            .expect("already final");
        assert!(dir.path().join("file_0.dat").exists());
    }
}
