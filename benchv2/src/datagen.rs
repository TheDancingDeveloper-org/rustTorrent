use crate::bencode::BValue;
use crate::config::{self, MB, GB};
use anyhow::Result;
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tokio::task;

fn piece_length(file_size: u64) -> u64 {
    if file_size < 128 * MB {
        256 * 1024
    } else if file_size < 512 * MB {
        512 * 1024
    } else if file_size < 2 * GB {
        MB
    } else if file_size < 8 * GB {
        2 * MB
    } else {
        4 * MB
    }
}

/// Generate a test file with random content. Shows progress for large files.
pub async fn generate_test_file(path: &Path, size: u64) -> Result<()> {
    let path = path.to_path_buf();
    task::spawn_blocking(move || {
        use rand::Rng;
        use std::io::Write;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::io::BufWriter::with_capacity(4 * MB as usize, std::fs::File::create(&path)?);
        let mut rng = rand::rng();
        let chunk_size = 4 * MB as usize;
        let mut buf = vec![0u8; chunk_size];
        let mut written: u64 = 0;
        let start = std::time::Instant::now();

        while written < size {
            let n = std::cmp::min(chunk_size as u64, size - written) as usize;
            rng.fill_bytes(&mut buf[..n]);
            f.write_all(&buf[..n])?;
            written += n as u64;
            if size >= GB && written % (256 * MB) == 0 {
                let pct = written as f64 * 100.0 / size as f64;
                let rate = written as f64 / MB as f64 / start.elapsed().as_secs_f64();
                eprint!("\r    {}: {:.0}% ({:.0} MB/s)", path.display(), pct, rate);
            }
        }
        f.flush()?;
        if size >= GB {
            eprintln!();
        }
        let elapsed = start.elapsed().as_secs_f64();
        tracing::info!(
            "Generated {} ({} MB) in {:.1}s",
            path.display(),
            size / MB,
            elapsed
        );
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    Ok(())
}

/// Create a .torrent file and return (torrent_bytes, info_hash_hex).
pub async fn create_torrent(data_path: &Path, tracker_url: &str) -> Result<(Vec<u8>, String)> {
    let data_path = data_path.to_path_buf();
    let tracker_url = tracker_url.to_string();

    task::spawn_blocking(move || {
        let meta = std::fs::metadata(&data_path)?;
        let file_size = meta.len();
        let pl = piece_length(file_size);
        let file_name = data_path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("no file name"))?
            .to_string_lossy();

        // Hash all pieces
        let mut pieces = Vec::new();
        let mut f = std::io::BufReader::new(std::fs::File::open(&data_path)?);
        let mut buf = vec![0u8; pl as usize];
        loop {
            use std::io::Read;
            let mut read_total = 0;
            while read_total < pl as usize {
                let n = f.read(&mut buf[read_total..])?;
                if n == 0 {
                    break;
                }
                read_total += n;
            }
            if read_total == 0 {
                break;
            }
            let hash = Sha1::digest(&buf[..read_total]);
            pieces.extend_from_slice(&hash);
        }

        let mut info = BTreeMap::new();
        info.insert(b"length".to_vec(), BValue::Int(file_size as i64));
        info.insert(b"name".to_vec(), BValue::Bytes(file_name.as_bytes().to_vec()));
        info.insert(b"piece length".to_vec(), BValue::Int(pl as i64));
        info.insert(b"pieces".to_vec(), BValue::Bytes(pieces));

        let info_bval = BValue::Dict(info.clone());
        let info_encoded = info_bval.encode();
        let info_hash = Sha1::digest(&info_encoded);
        let info_hash_hex: String = info_hash
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect();

        let mut torrent = BTreeMap::new();
        torrent.insert(
            b"announce".to_vec(),
            BValue::Bytes(tracker_url.as_bytes().to_vec()),
        );
        torrent.insert(
            b"created by".to_vec(),
            BValue::Bytes(b"benchv2".to_vec()),
        );
        torrent.insert(
            b"creation date".to_vec(),
            BValue::Int(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs() as i64,
            ),
        );
        torrent.insert(b"info".to_vec(), BValue::Dict(info));

        let torrent_bytes = BValue::Dict(torrent).encode();
        Ok((torrent_bytes, info_hash_hex))
    })
    .await?
}

/// File name for a given size label and index.
pub fn data_filename(label: &str, idx: usize) -> String {
    format!("bench_{}_{:03}.bin", label, idx)
}

pub fn torrent_filename(label: &str, idx: usize) -> String {
    format!("bench_{}_{:03}.torrent", label, idx)
}

/// Prepare all test data and torrent files needed for the given scenarios.
pub async fn prepare_data(
    scenarios: &[config::Scenario],
    data_dir: &Path,
    torrent_dir: &Path,
) -> Result<()> {
    use std::collections::HashMap;

    tokio::fs::create_dir_all(data_dir).await?;
    tokio::fs::create_dir_all(torrent_dir).await?;

    // Determine unique (size -> max file count) needed
    let mut needed: HashMap<u64, usize> = HashMap::new();
    for sc in scenarios {
        let entry = needed.entry(sc.file_size).or_insert(0);
        *entry = std::cmp::max(*entry, sc.num_files);
    }

    let total_gen: u64 = needed.iter().map(|(&sz, &cnt)| sz * cnt as u64).sum();
    tracing::info!(
        "Need {} unique size(s), up to {:.1} GB total",
        needed.len(),
        total_gen as f64 / GB as f64
    );

    for (&size, &count) in &needed {
        let label = config::size_label(size);
        tracing::info!("{}: {} file(s) of {} MB each", label, count, size / MB);

        for i in 0..count {
            let fpath = data_dir.join(data_filename(&label, i));
            let tpath = torrent_dir.join(torrent_filename(&label, i));

            // Generate data file if missing or wrong size
            let needs_gen = match tokio::fs::metadata(&fpath).await {
                Ok(m) => m.len() != size,
                Err(_) => true,
            };
            if needs_gen {
                tracing::info!("  Generating {}...", fpath.display());
                generate_test_file(&fpath, size).await?;
            }

            // Create torrent if missing
            if !tpath.exists() {
                tracing::info!("  Creating torrent {}...", tpath.display());
                let (torrent_bytes, hash) =
                    create_torrent(&fpath, config::TRACKER_ANNOUNCE).await?;
                tokio::fs::write(&tpath, &torrent_bytes).await?;
                tracing::info!("    info_hash: {}", hash);
            }
        }
    }
    tracing::info!("Test data ready.");
    Ok(())
}

/// Get torrent file paths for a scenario.
pub fn torrent_paths(sc: &config::Scenario, torrent_dir: &Path) -> Vec<PathBuf> {
    let label = config::size_label(sc.file_size);
    (0..sc.num_files)
        .map(|i| torrent_dir.join(torrent_filename(&label, i)))
        .collect()
}
