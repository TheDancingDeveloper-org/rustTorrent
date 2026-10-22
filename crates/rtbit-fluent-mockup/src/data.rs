//! Fake data: about 200 torrents with names, sizes, states, speeds, peers, trackers, file trees
//! and piece bitfields. Everything is generated from a fixed seed, so every run looks the same.
//! [`Library::tick`] moves the speeds and progress a little each second.

use std::collections::VecDeque;

/// Number of fake torrents.
pub const TORRENT_COUNT: usize = 200;
/// Samples kept for the speed sparkline (one per tick).
pub const HISTORY_LEN: usize = 90;

/// Small deterministic PRNG (xorshift64*), so the mockup needs no `rand`.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Uniform in `0.0..1.0`.
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }
}

/// Torrent state, as the web UI's status filter names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum State {
    Downloading,
    Seeding,
    Paused,
    Queued,
    Checking,
    Stalled,
    Error,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            State::Downloading => "Downloading",
            State::Seeding => "Seeding",
            State::Paused => "Paused",
            State::Queued => "Queued",
            State::Checking => "Checking",
            State::Stalled => "Stalled",
            State::Error => "Error",
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, State::Downloading | State::Seeding | State::Checking)
    }
}

/// One piece in the bitfield.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece {
    Missing,
    Downloading,
    Have,
}

#[derive(Clone, Debug)]
pub struct Peer {
    pub addr: String,
    pub client: String,
    pub flags: &'static str,
    pub country: &'static str,
    pub progress: f32,
    pub down: f64,
    pub up: f64,
    pub downloaded: u64,
    pub uploaded: u64,
}

#[derive(Clone, Debug)]
pub struct Tracker {
    pub url: String,
    pub tier: u32,
    pub status: &'static str,
    pub seeds: u32,
    pub leechers: u32,
    pub downloaded: u32,
    pub message: &'static str,
    pub next_announce: u32,
}

/// A node in a torrent's file tree. Folders have children; files have none.
#[derive(Clone, Debug)]
pub struct FileNode {
    pub name: String,
    pub size: u64,
    pub progress: f32,
    pub priority: &'static str,
    pub children: Vec<FileNode>,
}

impl FileNode {
    fn folder(name: impl Into<String>, children: Vec<FileNode>) -> Self {
        let size = children.iter().map(|c| c.size).sum();
        let progress = if size == 0 {
            0.0
        } else {
            children
                .iter()
                .map(|c| c.progress * c.size as f32)
                .sum::<f32>()
                / size as f32
        };
        Self {
            name: name.into(),
            size,
            progress,
            priority: "Normal",
            children,
        }
    }

    /// Find a node by its path of child indices.
    pub fn at(&self, path: &[usize]) -> Option<&FileNode> {
        let mut node = self;
        for &i in path {
            node = node.children.get(i)?;
        }
        Some(node)
    }
}

#[derive(Clone, Debug)]
pub struct Torrent {
    pub id: u64,
    pub name: String,
    pub info_hash: String,
    pub size: u64,
    pub progress: f32,
    pub state: State,
    pub down: f64,
    pub up: f64,
    pub seeds: u32,
    pub seeds_total: u32,
    pub peers: u32,
    pub peers_total: u32,
    pub ratio: f32,
    pub uploaded: u64,
    pub category: &'static str,
    pub tracker_host: &'static str,
    /// Days before "now" (2026-10-22) that the torrent was added.
    pub added_days_ago: u32,
    /// Minute of the day it was added.
    pub added_minute: u32,
    pub save_path: String,
    pub piece_size: u64,
    pub pieces: Vec<Piece>,
    pub files: FileNode,
    pub peer_list: Vec<Peer>,
    pub trackers: Vec<Tracker>,
    pub down_history: VecDeque<f32>,
    pub up_history: VecDeque<f32>,
    pub error: Option<&'static str>,
    pub queue_position: Option<u32>,
    pub sequential: bool,
    pub super_seed: bool,
    pub seed_time_secs: u64,
    pub availability: f32,
    /// Target speeds the animation drifts around.
    base_down: f64,
    base_up: f64,
}

impl Torrent {
    pub fn eta_secs(&self) -> Option<u64> {
        if self.state != State::Downloading || self.down < 1.0 {
            return None;
        }
        let left = self.size as f64 * (1.0 - self.progress as f64);
        Some((left / self.down) as u64)
    }

    /// "MM/DD/YY HH:MM", as the Added On column shows it.
    pub fn added_on(&self) -> String {
        // Days from 2026-10-22 back, on a 365-day calendar with real month lengths.
        const MONTHS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let (mut year, mut month, mut day) = (2026u32, 10usize, 22u32);
        for _ in 0..self.added_days_ago {
            if day > 1 {
                day -= 1;
            } else {
                if month == 1 {
                    month = 12;
                    year -= 1;
                } else {
                    month -= 1;
                }
                day = MONTHS[month - 1];
            }
        }
        format!(
            "{month:02}/{day:02}/{:02} {:02}:{:02}",
            year % 100,
            self.added_minute / 60,
            self.added_minute % 60
        )
    }

    pub fn downloaded(&self) -> u64 {
        (self.size as f64 * self.progress as f64) as u64
    }
}

pub const CATEGORIES: [&str; 6] = ["Movies", "TV", "Music", "Linux ISOs", "Books", "Software"];
pub const TRACKERS: [&str; 6] = [
    "tracker.opentrackr.org",
    "open.stealth.si",
    "tracker.torrent.eu.org",
    "exodus.desync.com",
    "tracker.debian.org",
    "torrent.ubuntu.com",
];

const LINUX: [&str; 10] = [
    "ubuntu-24.04.3-desktop-amd64.iso",
    "debian-13.1.0-amd64-netinst.iso",
    "Fedora-Workstation-Live-x86_64-43",
    "archlinux-2026.10.01-x86_64.iso",
    "linuxmint-22.2-cinnamon-64bit.iso",
    "openSUSE-Tumbleweed-DVD-x86_64-Current",
    "kali-linux-2026.3-installer-amd64",
    "manjaro-kde-25.0.4-minimal",
    "pop-os_24.04_amd64_nvidia",
    "NixOS-25.05-gnome-x86_64",
];
const FILM: [&str; 16] = [
    "Big.Buck.Bunny",
    "Sintel",
    "Tears.of.Steel",
    "Elephants.Dream",
    "Cosmos.Laundromat",
    "Spring",
    "Agent.327.Operation.Barbershop",
    "Caminandes.Llamigos",
    "Glass.Half",
    "Hero",
    "Charge",
    "Coffee.Run",
    "Sprite.Fright",
    "Wing.It",
    "The.Daily.Dweebs",
    "Night.of.the.Living.Dead",
];
const SHOW: [&str; 8] = [
    "Pioneer.One",
    "The.Lost.Lecture.Series",
    "Open.Source.Stories",
    "Archive.Hour",
    "Public.Domain.Theatre",
    "Night.Sky.Diaries",
    "Rail.Journeys",
    "Kitchen.Lab",
];
const MUSIC: [&str; 10] = [
    "Kevin MacLeod - Royalty Free Collection",
    "Jamendo Top 100 - 2026",
    "Musopen - Chopin Nocturnes",
    "Free Music Archive - Ambient Vol. 4",
    "Nine Inch Nails - The Slip",
    "Brad Sucks - Out of It",
    "Josh Woodward - Breadcrumbs",
    "Chris Zabriskie - Divider",
    "Podington Bear - Soundtracks",
    "Lee Rosevere - Music for Podcasts",
];
const BOOKS: [&str; 8] = [
    "Project Gutenberg Top 100 (EPUB)",
    "The Rust Programming Language (2026 ed)",
    "Structure and Interpretation of Computer Programs",
    "Wikipedia Offline - en - 2026-09",
    "Pro Git 2nd Edition",
    "Think Python 3e",
    "Feynman Lectures - Vol 1-3",
    "Internet Archive - Pulp Magazines 1935",
];
const SOFTWARE: [&str; 8] = [
    "LibreOffice_25.8.2_Linux_x86-64_deb",
    "Blender-4.5.3-linux-x64",
    "GIMP-3.0.6-setup",
    "Krita-5.3.1-x86_64.appimage",
    "OBS-Studio-32.0-Full-Installer",
    "Godot_v4.5-stable_linux.x86_64",
    "Inkscape-1.4.2-x64",
    "KiCad-9.0.4-x86_64",
];
const CLIENTS: [&str; 8] = [
    "rustTorrent 0.2.0",
    "Transmission 4.0.6",
    "Deluge 2.1.1",
    "libtorrent 2.0.11",
    "BiglyBT 3.8",
    "rqbit 8.1",
    "aria2 1.37",
    "uTorrent 3.6",
];
const COUNTRIES: [&str; 10] = ["AU", "DE", "US", "NL", "FR", "SE", "CA", "JP", "BR", "GB"];
const FLAGS: [&str; 6] = ["D U E", "d U", "D u E X", "U", "D H", "u I"];

const KIB: u64 = 1024;
const MIB: u64 = KIB * 1024;
const GIB: u64 = MIB * 1024;

fn hex_hash(rng: &mut Rng) -> String {
    (0..5)
        .map(|_| format!("{:08x}", rng.next_u64() as u32))
        .collect()
}

fn file_tree(rng: &mut Rng, name: &str, category: &str, total: u64) -> FileNode {
    let mut files = Vec::new();
    let file = |name: String, size: u64, rng: &mut Rng| FileNode {
        name,
        size,
        progress: if rng.chance(0.4) { 1.0 } else { rng.f32() },
        priority: rng.pick(&["Normal", "Normal", "Normal", "High", "Low", "Skip"]),
        children: Vec::new(),
    };
    match category {
        "TV" => {
            for season in 1..=2 {
                let mut eps = Vec::new();
                for ep in 1..=6 {
                    eps.push(file(
                        format!("{name}.S{season:02}E{ep:02}.1080p.WEB.x265.mkv"),
                        total / 14,
                        rng,
                    ));
                }
                files.push(FileNode::folder(format!("Season {season}"), eps));
            }
            files.push(file("Extras/Behind.the.Scenes.mkv".into(), total / 14, rng));
        }
        "Music" => {
            let mut tracks = Vec::new();
            for t in 1..=10 {
                tracks.push(file(format!("{t:02} - Track {t}.flac"), total / 11, rng));
            }
            files.push(FileNode::folder("CD1", tracks));
            files.push(file("cover.jpg".into(), total / 11, rng));
        }
        "Books" | "Software" => {
            files.push(file(format!("{name}.zip"), total * 9 / 10, rng));
            files.push(file("README.txt".into(), 4 * KIB, rng));
            files.push(file("checksums.sha256".into(), KIB, rng));
        }
        _ => {
            files.push(file(format!("{name}.mkv"), total * 95 / 100, rng));
            files.push(FileNode::folder(
                "Subs",
                vec![
                    file("English.srt".into(), 80 * KIB, rng),
                    file("German.srt".into(), 78 * KIB, rng),
                    file("Spanish.srt".into(), 81 * KIB, rng),
                ],
            ));
            files.push(file("sample.mkv".into(), total / 25, rng));
        }
    }
    FileNode::folder(name.to_owned(), files)
}

fn pieces(rng: &mut Rng, count: usize, progress: f32, state: State) -> Vec<Piece> {
    // Runs of have/missing look more like a real swarm than per-piece noise.
    let mut out = Vec::with_capacity(count);
    let mut have = rng.chance(progress);
    while out.len() < count {
        let run = 1 + rng.below(count / 20 + 2);
        for _ in 0..run.min(count - out.len()) {
            out.push(if have { Piece::Have } else { Piece::Missing });
        }
        have = rng.chance(progress);
    }
    if state == State::Downloading {
        for _ in 0..(count / 40).max(3) {
            let i = rng.below(count);
            if out[i] == Piece::Missing {
                out[i] = Piece::Downloading;
            }
        }
    }
    if progress >= 1.0 {
        out.fill(Piece::Have);
    }
    out
}

fn torrent(id: u64, rng: &mut Rng) -> Torrent {
    let category = *rng.pick(&CATEGORIES);
    // About one in eight has no category, for the sidebar's "Uncategorized".
    let shown_category = if rng.chance(0.12) { "" } else { category };
    let (name, size) = match category {
        "Linux ISOs" => (rng.pick(&LINUX).to_string(), (rng.f32() * 5.0 + 0.6) as f64),
        "Movies" => (
            format!(
                "{}.{}.{}",
                rng.pick(&FILM),
                2008 + rng.below(18),
                rng.pick(&[
                    "1080p.BluRay.x264",
                    "2160p.WEB-DL.HEVC",
                    "720p.HDTV",
                    "REMUX.AVC"
                ])
            ),
            (rng.f32() * 40.0 + 1.0) as f64,
        ),
        "TV" => (
            format!("{}.Complete.1080p.WEB.x265", rng.pick(&SHOW)),
            (rng.f32() * 20.0 + 2.0) as f64,
        ),
        "Music" => (
            format!("{} [FLAC]", rng.pick(&MUSIC)),
            (rng.f32() * 1.5 + 0.2) as f64,
        ),
        "Books" => (
            rng.pick(&BOOKS).to_string(),
            (rng.f32() * 0.8 + 0.01) as f64,
        ),
        _ => (
            rng.pick(&SOFTWARE).to_string(),
            (rng.f32() * 3.0 + 0.1) as f64,
        ),
    };
    let size = (size * GIB as f64) as u64;
    let state = *rng.pick(&[
        State::Downloading,
        State::Downloading,
        State::Downloading,
        State::Seeding,
        State::Seeding,
        State::Seeding,
        State::Seeding,
        State::Paused,
        State::Paused,
        State::Queued,
        State::Checking,
        State::Stalled,
        State::Error,
    ]);
    let progress = match state {
        State::Seeding => 1.0,
        State::Queued => 0.0,
        _ if rng.chance(0.15) => 1.0,
        _ => rng.f32() * 0.98,
    };
    let (base_down, base_up) = match state {
        State::Downloading => (
            rng.f32() as f64 * 12.0 * MIB as f64,
            rng.f32() as f64 * 900.0 * KIB as f64,
        ),
        State::Seeding => (
            0.0,
            if rng.chance(0.6) {
                rng.f32() as f64 * 2.0 * MIB as f64
            } else {
                0.0
            },
        ),
        State::Checking => (0.0, 0.0),
        _ => (0.0, 0.0),
    };
    let piece_size = if size > 8 * GIB {
        16 * MIB
    } else if size > GIB {
        4 * MIB
    } else {
        MIB
    };
    let piece_count = (size / piece_size).clamp(64, 2400) as usize;
    let tracker_host = *rng.pick(&TRACKERS);
    let seeds_total = rng.below(900) as u32 + 1;
    let peers_total = rng.below(300) as u32;
    let active = state.is_active() || state == State::Stalled;
    let seeds = if active && state != State::Seeding {
        rng.below(seeds_total.min(60) as usize) as u32
    } else {
        0
    };
    let peers = if active {
        rng.below(peers_total.min(40) as usize + 1) as u32
    } else {
        0
    };
    let ratio = if progress >= 1.0 {
        rng.f32() * 4.0
    } else {
        rng.f32() * 0.6
    };
    let peer_list = (0..(seeds + peers).min(40))
        .map(|_| Peer {
            addr: format!(
                "{}.{}.{}.{}:{}",
                rng.below(223) + 1,
                rng.below(255),
                rng.below(255),
                rng.below(254) + 1,
                1024 + rng.below(60_000)
            ),
            client: rng.pick(&CLIENTS).to_string(),
            flags: rng.pick(&FLAGS),
            country: rng.pick(&COUNTRIES),
            progress: if rng.chance(0.5) { 1.0 } else { rng.f32() },
            down: if base_down > 0.0 {
                rng.f32() as f64 * base_down / 4.0
            } else {
                0.0
            },
            up: if base_up > 0.0 {
                rng.f32() as f64 * base_up / 4.0
            } else {
                0.0
            },
            downloaded: rng.below(800) as u64 * MIB,
            uploaded: rng.below(300) as u64 * MIB,
        })
        .collect();
    let mut trackers = vec![Tracker {
        url: format!("udp://{tracker_host}:1337/announce"),
        tier: 0,
        status: if state == State::Error {
            "Not working"
        } else {
            "Working"
        },
        seeds: seeds_total,
        leechers: peers_total,
        downloaded: rng.below(40_000) as u32,
        message: if state == State::Error {
            "Connection timed out"
        } else {
            ""
        },
        next_announce: rng.below(1800) as u32,
    }];
    for (tier, host) in [(1, *rng.pick(&TRACKERS)), (2, "tracker.example.invalid")] {
        if host != tracker_host {
            let working = !host.ends_with(".invalid");
            trackers.push(Tracker {
                url: format!("https://{host}/announce"),
                tier,
                status: if working { "Working" } else { "Not working" },
                seeds: if working { seeds_total / 2 } else { 0 },
                leechers: if working { peers_total / 2 } else { 0 },
                downloaded: 0,
                message: if working { "" } else { "Host not found" },
                next_announce: rng.below(1800) as u32,
            });
        }
    }
    let files = file_tree(rng, &name, category, size);
    let tracker_host = if rng.chance(0.05) { "" } else { tracker_host };
    if tracker_host.is_empty() {
        trackers.clear();
    }
    let history = |base: f64, rng: &mut Rng| -> VecDeque<f32> {
        (0..HISTORY_LEN)
            .map(|i| {
                let wave = ((i as f32) / 9.0).sin() * 0.25 + 1.0;
                (base as f32 * wave * (0.7 + rng.f32() * 0.6)).max(0.0)
            })
            .collect()
    };
    let down_history = history(base_down, rng);
    let up_history = history(base_up, rng);
    Torrent {
        id,
        info_hash: hex_hash(rng),
        size,
        progress,
        state,
        down: base_down,
        up: base_up,
        seeds,
        seeds_total,
        peers,
        peers_total,
        ratio,
        uploaded: (size as f64 * ratio as f64) as u64,
        category: shown_category,
        tracker_host,
        added_days_ago: rng.below(400) as u32,
        added_minute: rng.below(1440) as u32,
        save_path: format!(
            "/srv/downloads/{}",
            category.to_lowercase().replace(' ', "-")
        ),
        piece_size,
        pieces: pieces(rng, piece_count, progress, state),
        files,
        peer_list,
        trackers,
        down_history,
        up_history,
        error: (state == State::Error).then_some("Tracker: connection timed out; no peers"),
        queue_position: (state == State::Queued).then(|| rng.below(12) as u32 + 1),
        sequential: rng.chance(0.1),
        super_seed: false,
        seed_time_secs: if progress >= 1.0 {
            rng.below(40 * 86_400) as u64
        } else {
            0
        },
        availability: if active { 1.0 + rng.f32() * 30.0 } else { 0.0 },
        base_down,
        base_up,
        name,
    }
}

/// All fake torrents and the global counters the status bar shows.
pub struct Library {
    pub torrents: Vec<Torrent>,
    pub global_down: VecDeque<f32>,
    pub global_up: VecDeque<f32>,
    rng: Rng,
    next_id: u64,
}

impl Library {
    pub fn generate() -> Self {
        let mut rng = Rng::new(0x5eed_2160);
        let torrents: Vec<_> = (0..TORRENT_COUNT as u64)
            .map(|id| torrent(id, &mut rng))
            .collect();
        let mut lib = Self {
            torrents,
            global_down: VecDeque::new(),
            global_up: VecDeque::new(),
            rng,
            next_id: TORRENT_COUNT as u64,
        };
        for _ in 0..HISTORY_LEN {
            lib.tick();
        }
        lib
    }

    pub fn total_down(&self) -> f64 {
        self.torrents.iter().map(|t| t.down).sum()
    }

    pub fn total_up(&self) -> f64 {
        self.torrents.iter().map(|t| t.up).sum()
    }

    /// One animation step: speeds drift around their base, downloading torrents make progress.
    pub fn tick(&mut self) {
        let rng = &mut self.rng;
        for t in &mut self.torrents {
            let active = matches!(t.state, State::Downloading | State::Seeding);
            if active {
                let jitter =
                    |base: f64, rng: &mut Rng| (base * (0.75 + rng.f32() as f64 * 0.5)).max(0.0);
                t.down = jitter(t.base_down, rng);
                t.up = jitter(t.base_up, rng);
            } else {
                t.down = 0.0;
                t.up = 0.0;
            }
            if t.state == State::Downloading && t.size > 0 {
                t.progress = (t.progress + (t.down / t.size as f64) as f32).min(1.0);
                // Flip a few missing pieces to have, so the bitfield moves too.
                let n = t.pieces.len();
                for _ in 0..2 {
                    let i = rng.below(n);
                    if t.pieces[i] != Piece::Have {
                        t.pieces[i] = if t.pieces[i] == Piece::Downloading {
                            Piece::Have
                        } else {
                            Piece::Downloading
                        };
                    }
                }
                if t.progress >= 1.0 {
                    t.state = State::Seeding;
                    t.base_down = 0.0;
                    t.pieces.fill(Piece::Have);
                }
            }
            push(&mut t.down_history, t.down as f32);
            push(&mut t.up_history, t.up as f32);
        }
        let (d, u) = (self.total_down() as f32, self.total_up() as f32);
        push(&mut self.global_down, d);
        push(&mut self.global_up, u);
    }

    pub fn by_id(&self, id: u64) -> Option<&Torrent> {
        self.torrents.iter().find(|t| t.id == id)
    }

    pub fn by_id_mut(&mut self, id: u64) -> Option<&mut Torrent> {
        self.torrents.iter_mut().find(|t| t.id == id)
    }

    /// Add a fake torrent named `name`, as the add dialog does.
    pub fn add(&mut self, name: &str, category: &'static str, paused: bool) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let mut t = torrent(id, &mut self.rng);
        t.name = name.to_owned();
        t.category = category;
        t.progress = 0.0;
        t.pieces.fill(Piece::Missing);
        t.added_days_ago = 0;
        if paused {
            t.state = State::Paused;
        } else {
            t.state = State::Downloading;
            t.base_down = 6.0 * MIB as f64;
            t.base_up = 200.0 * KIB as f64;
        }
        self.torrents.push(t);
        id
    }

    pub fn set_state(&mut self, ids: &[u64], state: State) {
        for id in ids {
            if let Some(t) = self.by_id_mut(*id) {
                t.state = if state == State::Downloading && t.progress >= 1.0 {
                    State::Seeding
                } else {
                    state
                };
                if t.state == State::Downloading && t.base_down == 0.0 {
                    t.base_down = 3.0 * MIB as f64;
                }
            }
        }
    }

    pub fn remove(&mut self, ids: &[u64]) {
        self.torrents.retain(|t| !ids.contains(&t.id));
    }
}

fn push(series: &mut VecDeque<f32>, v: f32) {
    series.push_back(v);
    while series.len() > HISTORY_LEN {
        series.pop_front();
    }
}

/// `1.4 GiB`, `512 KiB`.
pub fn bytes(n: u64) -> String {
    let n = n as f64;
    if n >= GIB as f64 {
        format!("{:.2} GiB", n / GIB as f64)
    } else if n >= MIB as f64 {
        format!("{:.1} MiB", n / MIB as f64)
    } else if n >= KIB as f64 {
        format!("{:.0} KiB", n / KIB as f64)
    } else {
        format!("{n:.0} B")
    }
}

/// `3.2 MiB/s`; empty for zero, as the web UI shows a blank cell.
pub fn speed(bps: f64) -> String {
    if bps < 1.0 {
        String::new()
    } else {
        format!("{}/s", bytes(bps as u64))
    }
}

/// `2h 14m`, `∞`.
pub fn eta(secs: Option<u64>) -> String {
    match secs {
        None => "∞".to_owned(),
        Some(s) if s >= 86_400 => format!("{}d {}h", s / 86_400, s % 86_400 / 3600),
        Some(s) if s >= 3600 => format!("{}h {}m", s / 3600, s % 3600 / 60),
        Some(s) if s >= 60 => format!("{}m {}s", s / 60, s % 60),
        Some(s) => format!("{s}s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_about_two_hundred_torrents() {
        let lib = Library::generate();
        assert_eq!(lib.torrents.len(), TORRENT_COUNT);
        assert!(lib.torrents.iter().any(|t| t.state == State::Downloading));
        assert!(lib.torrents.iter().all(|t| !t.pieces.is_empty()));
    }

    #[test]
    fn formats() {
        assert_eq!(bytes(1536), "2 KiB");
        assert_eq!(eta(Some(3700)), "1h 1m");
        assert_eq!(speed(0.0), "");
    }
}
