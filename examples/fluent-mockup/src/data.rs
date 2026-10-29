//! Deterministic fake torrents. Speeds and progress drift with the frame
//! clock so the mockup looks alive, but the set itself never changes.

/// One row in the torrent list.
#[derive(Clone, Debug)]
pub struct Torrent {
    pub id: u64,
    pub name: String,
    pub category: &'static str,
    pub tracker: &'static str,
    pub size: u64,
    /// 0.0 to 1.0 at rest, before the small animation offset.
    pub progress: f32,
    pub download_bps: u64,
    pub upload_bps: u64,
    pub peers: u32,
    pub seeds: u32,
    pub state: &'static str,
    /// Piece bits, length 64. `true` means the piece is present.
    pub pieces: Vec<bool>,
}

const NAMES: &[&str] = &[
    "debian-13.1.0-amd64-netinst",
    "rust-1.95.0-x86_64-unknown-linux-gnu",
    "blender-4.2-linux-x64",
    "libreoffice-25.2.0-linux",
    "kernel-6.12.0-source",
    "ubuntu-24.04.2-desktop-amd64",
    "archlinux-2026.03.01-x86_64",
    "gutenberg-dvd-2025",
    "wikipedia-en-2026-01",
    "dataset-osm-planet-2601",
    "music-archive-cc-2024",
    "conference-talks-2025",
];

const CATEGORIES: &[&str] = &["Linux", "Software", "Media", "Data"];
const TRACKERS: &[&str] = &["tracker.example", "mirror.example", "backup.example"];
const STATES: &[&str] = &["downloading", "seeding", "paused", "queued"];

/// Forty torrents, the same ones every run.
pub fn torrents() -> Vec<Torrent> {
    (0..40)
        .map(|index| {
            let name = format!("{}-{:02}", NAMES[index % NAMES.len()], index);
            let progress = if index % 7 == 0 {
                1.0
            } else {
                ((index * 17) % 100) as f32 / 100.0
            };
            let state = if progress >= 1.0 {
                "seeding"
            } else if index % 11 == 0 {
                "paused"
            } else {
                STATES[index % STATES.len()]
            };
            let mut pieces = Vec::with_capacity(64);
            for piece in 0..64 {
                let have = (index * 13 + piece * 7) % 11 != 0;
                pieces.push(have || progress >= 1.0);
            }
            Torrent {
                id: index as u64 + 1,
                name,
                category: CATEGORIES[index % CATEGORIES.len()],
                tracker: TRACKERS[index % TRACKERS.len()],
                size: 64 * 1024 * 1024 + (index as u64) * 180 * 1024 * 1024,
                progress,
                download_bps: if state == "downloading" {
                    200_000 + (index as u64) * 90_000
                } else {
                    0
                },
                upload_bps: if state == "paused" {
                    0
                } else {
                    40_000 + (index as u64) * 12_000
                },
                peers: 4 + (index as u32 % 40),
                seeds: 1 + (index as u32 % 20),
                state,
                pieces,
            }
        })
        .collect()
}

/// Bytes per second as a short label.
pub fn rate(bytes_per_second: u64) -> String {
    if bytes_per_second == 0 {
        return "—".to_owned();
    }
    let (value, unit) = if bytes_per_second >= 1_000_000 {
        (bytes_per_second as f64 / 1_000_000.0, "MB/s")
    } else {
        (bytes_per_second as f64 / 1_000.0, "KB/s")
    };
    format!("{value:.1} {unit}")
}

/// Size as a short label.
pub fn size(bytes: u64) -> String {
    let gib = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    if gib >= 1.0 {
        format!("{gib:.2} GiB")
    } else {
        format!("{:.0} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}
