// Mock API for testing UI with large number of torrents
// This file is only used in dev mode with mock.html entry point

import {
  AddTorrentResponse,
  AltSpeedConfig,
  AltSpeedSchedule,
  AltSpeedStatus,
  CategoryInfo,
  DhtStats,
  AdvancedConfig,
  ProxyConfig,
  IpFilterConfig,
  LimitsConfig,
  ListTorrentsResponse,
  PeerStatsSnapshot,
  RtbitAPI,
  SeedLimits,
  SeedLimitsConfig,
  SessionStats,
  TorrentDetails,
  TorrentLimits,
  TorrentStats,
  TorrentListItem,
  TrackerStatusEntry,
  TrackerStatusResponse,
  WebSeedEntry,
  LiveTorrentStats,
  TorrentFile,
  RssFeedConfig,
  RssItem,
  RssRule,
} from "./api-types";

// Deliberately fictional release names make the public demo feel populated
// without implying that any real torrent or tracker is being served.
const TORRENT_NAMES = [
  "NebulaOS-26.07-desktop-x86_64.iso",
  "CopperFinch-Linux-12.4-live-amd64.iso",
  "Atlas.Public.Dataset.2026.07.parquet.zst",
  "Moonbase.Manuals.Collection.v4.2.epub.zip",
  "Open.Orchestra.Sessions.Vol.08.FLAC",
  "Glass.River.2026.Short.Film.1080p.WEB.x264-DEMO",
  "Cinder.Workstation-9.1-arm64.img.xz",
  "Northstar.Game.Assets.CreativeCommons.v3.tar.zst",
  "Pocket.Planetarium.Catalog.2026-07.sqlite.zst",
  "Paper.Kites.Public.Domain.Collection.4K-DEMO",
  "Field.Notes.Quarterly.Issue.42.pdf.zip",
  "LighthouseOS-5.8-server-amd64.iso",
  "Mosaic.Fonts.Open.Collection.2026.1.tar.gz",
  "Signal.Garden.S01E04.1080p.WEB.x265-DEMO",
  "Blue.Hour.Live.Session.2026.FLAC-DEMO",
  "JuniperBSD-14.2-install-amd64.iso",
  "Rookery.Container.Images.airgap-bundle-v7.tar",
  "Cloud.Atlas.Weather.Archive.2026-06.nc.zst",
  "Tiny.Museums.Photo.Archive.Vol.12.zip",
  "Amber.Terminal-3.6.2-source-and-docs.tar.gz",
  "Fable.Engine.Sample.Projects.v2.8.zip",
  "Harbor.Light.2025.Open.Movie.2160p-DEMO",
  "OrchardOS-rolling-2026.07-kde-x86_64.iso",
  "Transit.Map.OpenData.Global.2026Q2.pbf",
  "Night.Train.Radio.Archive.Episodes.101-125.opus",
  "Stonecrop.Rescue.Environment.v5.1.iso",
  "Wildflower.Macro.Photos.CC0.Collection.03.zip",
  "CometDB-8.0.1-offline-documentation.tar.gz",
  "Riverside.Ambience.24bit.96kHz.FLAC-DEMO",
  "SundialOS-2.0-raspberrypi-arm64.img.xz",
  "Workshop.CAD.Models.Open-Pack.2026-07.zip",
  "Morning.Fog.2024.Short.Film.1080p-DEMO",
  "Redwood.Security.Lab.v11.qcow2",
  "Open.Cookbook.Archive.2026.07.epub",
  "Aurora.Language.Corpus.v6.jsonl.zst",
  "Telescope.Raw.Sample.Data.M31.2026.fits.tar",
];

// File name templates
const FILE_EXTENSIONS = [".iso", ".img", ".tar.gz", ".zip", ".qcow2"];

// Deliberately fictional tracker hosts for the public demo
const MOCK_TRACKER_URLS = [
  "udp://tracker.demo-open.example:6969/announce",
  "https://announce.publicbits.example:443/announce",
  "udp://tr.linuxmirror.example:2710/announce",
  "https://tracker.datasets.example/announce",
  "udp://open.seedhub.example:6969/announce",
];

// Assign 1-3 deterministic trackers per torrent
function generateTrackers(id: number): string[] {
  const rand = seededRandom(id * 6659);
  const count = 1 + Math.floor(rand() * 3);
  const urls = new Set<string>();
  for (let i = 0; i < count; i++) {
    urls.add(MOCK_TRACKER_URLS[Math.floor(rand() * MOCK_TRACKER_URLS.length)]);
  }
  return Array.from(urls);
}

function generateAddedOn(id: number): number {
  const rand = seededRandom(id * 7717);
  // Added sometime in the last 90 days
  return Math.floor(Date.now() / 1000 - rand() * 90 * 86400);
}

// Generate deterministic random number from seed
function seededRandom(seed: number): () => number {
  return () => {
    seed = (seed * 1103515245 + 12345) & 0x7fffffff;
    return seed / 0x7fffffff;
  };
}

// Generate a fake info_hash from id
function generateInfoHash(id: number): string {
  const chars = "0123456789abcdef";
  const rand = seededRandom(id * 31337);
  let hash = "";
  for (let i = 0; i < 40; i++) {
    hash += chars[Math.floor(rand() * 16)];
  }
  return hash;
}

// Generate torrent name from id
function generateTorrentName(id: number): string {
  return TORRENT_NAMES[id % TORRENT_NAMES.length];
}

// State weights for distribution
type TorrentState = "live" | "paused" | "initializing" | "error";

// Limit concurrent active torrents to be more realistic
const MAX_CONCURRENT_ACTIVE = 8;

function generateState(id: number): TorrentState {
  // Only first MAX_CONCURRENT_ACTIVE torrents are active by default
  // The rest are paused with some errors mixed in
  if (id < MAX_CONCURRENT_ACTIVE) {
    const rand = seededRandom(id * 7919);
    const r = rand();
    if (r < 0.85) return "live";
    if (r < 0.95) return "initializing";
    return "error";
  } else {
    // Most are paused, few have errors
    const rand = seededRandom(id * 7919);
    const r = rand();
    if (r < 0.95) return "paused";
    return "error";
  }
}

// Generate file size in bytes (500MB to 10GB)
function generateTotalBytes(id: number): number {
  const rand = seededRandom(id * 1013);
  const minSize = 500 * 1024 * 1024; // 500MB
  const maxSize = 10 * 1024 * 1024 * 1024; // 10GB
  return Math.floor(rand() * (maxSize - minSize) + minSize);
}

// Track progress over time for live torrents
const progressTracker = new Map<number, number>();

function getProgressBytes(
  id: number,
  totalBytes: number,
  state: TorrentState,
): number {
  if (state === "initializing") return 0;

  let progress = progressTracker.get(id);
  if (progress === undefined) {
    // Initialize with random progress
    const rand = seededRandom(id * 2749);
    progress = rand() * totalBytes;
    progressTracker.set(id, progress);
  }

  // Simulate progress for live torrents
  if (state === "live" && progress < totalBytes) {
    const increment = Math.random() * 5 * 1024 * 1024; // Up to 5MB per poll
    progress = Math.min(progress + increment, totalBytes);
    progressTracker.set(id, progress);
  }

  return Math.floor(progress);
}

// Generate files for a torrent
function generateFiles(id: number, totalBytes: number): TorrentFile[] {
  const rand = seededRandom(id * 4231);
  const name = generateTorrentName(id);
  const numFiles = Math.floor(rand() * 5) + 1; // 1-5 files

  const files: TorrentFile[] = [];
  let remainingBytes = totalBytes;

  for (let i = 0; i < numFiles; i++) {
    const isLast = i === numFiles - 1;
    const fileSize = isLast
      ? remainingBytes
      : Math.floor(rand() * remainingBytes * 0.7);
    remainingBytes -= fileSize;

    const ext = FILE_EXTENSIONS[Math.floor(rand() * FILE_EXTENSIONS.length)];
    const fileName =
      numFiles === 1 ? `${name}${ext}` : `${name}.part${i + 1}${ext}`;

    files.push({
      name: fileName,
      components: [fileName],
      length: fileSize,
      included: rand() > 0.1, // 90% included
      attributes: {
        symlink: false,
        hidden: false,
        padding: false,
        executable: false,
      },
    });
  }

  return files;
}

// Generate live stats
function generateLiveStats(
  id: number,
  progressBytes: number,
  totalBytes: number,
): LiveTorrentStats {
  const rand = seededRandom(id * 8737 + (Date.now() % 10000));
  const remainingBytes = totalBytes - progressBytes;
  const downloadSpeed = Math.random() * 50; // 0-50 Mbps
  const uploadSpeed = Math.random() * 10; // 0-10 Mbps

  const downloadBytesPerSec = (downloadSpeed * 1024 * 1024) / 8;
  const etaSecs =
    downloadBytesPerSec > 0 ? remainingBytes / downloadBytesPerSec : null;

  return {
    snapshot: {
      have_bytes: progressBytes,
      downloaded_and_checked_bytes: progressBytes,
      downloaded_and_checked_pieces: Math.floor(
        (progressBytes / totalBytes) * 1000,
      ),
      fetched_bytes: progressBytes,
      uploaded_bytes: Math.floor(rand() * progressBytes * 0.5),
      initially_needed_bytes: totalBytes,
      remaining_bytes: remainingBytes,
      total_bytes: totalBytes,
      total_piece_download_ms: Math.floor(rand() * 100000),
      peer_stats: {
        queued: Math.floor(rand() * 50),
        connecting: Math.floor(rand() * 10),
        live: Math.floor(rand() * 30) + 1,
        seen: Math.floor(rand() * 200),
        dead: Math.floor(rand() * 100),
        not_needed: Math.floor(rand() * 20),
      },
    },
    average_piece_download_time: {
      secs: Math.floor(rand() * 2),
      nanos: Math.floor(rand() * 1000000000),
    },
    download_speed: {
      mbps: downloadSpeed,
      human_readable: `${downloadSpeed.toFixed(1)} MB/s`,
    },
    upload_speed: {
      mbps: uploadSpeed,
      human_readable: `${uploadSpeed.toFixed(1)} MB/s`,
    },
    all_time_download_speed: {
      mbps: downloadSpeed * 0.8,
      human_readable: `${(downloadSpeed * 0.8).toFixed(1)} MB/s`,
    },
    time_remaining:
      etaSecs !== null
        ? {
            human_readable:
              etaSecs < 60
                ? `${Math.floor(etaSecs)}s`
                : etaSecs < 3600
                  ? `${Math.floor(etaSecs / 60)}m`
                  : `${Math.floor(etaSecs / 3600)}h ${Math.floor((etaSecs % 3600) / 60)}m`,
            duration: { secs: Math.floor(etaSecs) },
          }
        : null,
  };
}

// Generate torrent stats
function generateTorrentStats(id: number): TorrentStats {
  const state = torrentStates.get(id) ?? generateState(id);
  const totalBytes = generateTotalBytes(id);
  const progressBytes = getProgressBytes(id, totalBytes, state);
  const finished = progressBytes >= totalBytes;

  const rand = seededRandom(id * 5501);
  const numFiles = Math.floor(rand() * 5) + 1;
  const fileProgress = Array(numFiles)
    .fill(0)
    .map(() => (finished ? 1 : rand() * (progressBytes / totalBytes)));

  return {
    state: finished && state === "live" ? "live" : state,
    error: state === "error" ? "Connection timed out" : null,
    file_progress: fileProgress,
    progress_bytes: progressBytes,
    finished,
    total_bytes: totalBytes,
    live:
      state === "live"
        ? generateLiveStats(id, progressBytes, totalBytes)
        : null,
  };
}

// Generate torrent list item
function generateTorrentListItem(
  id: number,
  withStats: boolean,
): TorrentListItem {
  const totalBytes = generateTotalBytes(id);
  const totalPieces = Math.ceil(totalBytes / (256 * 1024)); // 256KB pieces

  const item: TorrentListItem = {
    id,
    info_hash: generateInfoHash(id),
    name: generateTorrentName(id),
    output_folder: `/downloads/torrent_${id}`,
    total_pieces: totalPieces,
    added_on: generateAddedOn(id),
    trackers: generateTrackers(id),
  };

  const category = torrentCategories.get(id);
  if (category) {
    item.category = category;
  }
  const tags = torrentTags.get(id);
  if (tags && tags.size > 0) {
    item.tags = Array.from(tags).sort();
  }

  if (withStats) {
    item.stats = generateTorrentStats(id);
  }

  return item;
}

// Store for tracking torrent state changes
const torrentStates = new Map<number, TorrentState>();
const deletedTorrents = new Set<number>();

// Mock alternative speed limits state
const mockAltSpeed = {
  enabled: false,
  config: {
    download_rate: 2 * 1024 * 1024,
    upload_rate: 512 * 1024,
  } as AltSpeedConfig,
  schedule: {
    enabled: false,
    start_minutes: 22 * 60,
    end_minutes: 6 * 60,
    days: 127,
  } as AltSpeedSchedule,
};

// Store stable peer data per torrent
interface PeerData {
  ip: string;
  port: number;
  connKind: "tcp" | "utp";
  // Counters that grow over time
  fetchedBytes: number;
  uploadedBytes: number;
  fetchRate: number; // bytes per second baseline
  uploadRate: number;
}

const torrentPeers = new Map<number, PeerData[]>();
const peerLastUpdate = new Map<number, number>();

function getOrCreatePeers(torrentId: number): PeerData[] {
  let peers = torrentPeers.get(torrentId);
  if (!peers) {
    const rand = seededRandom(torrentId * 9371);
    const numPeers = Math.floor(rand() * 15) + 5; // 5-20 peers
    peers = [];

    for (let i = 0; i < numPeers; i++) {
      peers.push({
        ip: `${Math.floor(rand() * 256)}.${Math.floor(rand() * 256)}.${Math.floor(rand() * 256)}.${Math.floor(rand() * 256)}`,
        port: 6881 + Math.floor(rand() * 1000),
        connKind: rand() > 0.3 ? "tcp" : "utp",
        fetchedBytes: Math.floor(rand() * 10000000), // Initial bytes
        uploadedBytes: Math.floor(rand() * 5000000),
        fetchRate: Math.floor(rand() * 2000000) + 100000, // 100KB-2MB/s
        uploadRate: Math.floor(rand() * 500000) + 50000, // 50KB-500KB/s
      });
    }
    torrentPeers.set(torrentId, peers);
    peerLastUpdate.set(torrentId, Date.now());
  }
  return peers;
}

function updatePeerCounters(torrentId: number): void {
  const peers = torrentPeers.get(torrentId);
  const lastUpdate = peerLastUpdate.get(torrentId);
  if (!peers || !lastUpdate) return;

  const now = Date.now();
  const elapsed = (now - lastUpdate) / 1000; // seconds
  peerLastUpdate.set(torrentId, now);

  // Only update if torrent is live
  const state = torrentStates.get(torrentId) ?? generateState(torrentId);
  if (state !== "live") return;

  for (const peer of peers) {
    // Add some variance to the rates
    const fetchVariance = 0.5 + Math.random();
    const uploadVariance = 0.5 + Math.random();
    peer.fetchedBytes += Math.floor(peer.fetchRate * elapsed * fetchVariance);
    peer.uploadedBytes += Math.floor(
      peer.uploadRate * elapsed * uploadVariance,
    );
  }
}

const TOTAL_TORRENTS = TORRENT_NAMES.length;

// Mock category data
const MOCK_CATEGORY_NAMES = ["Linux ISOs", "Software", "Documents", "Media"];

const mockCategories: Record<string, CategoryInfo> = {
  "Linux ISOs": { name: "Linux ISOs", save_path: "/downloads/linux" },
  Software: { name: "Software", save_path: "/downloads/software" },
  Documents: { name: "Documents", save_path: "/downloads/docs" },
  Media: { name: "Media", save_path: "/downloads/media" },
};

// Track torrent category assignments
const torrentCategories = new Map<number, string>();

// Tag registry and per-torrent assignments for the mock.
const mockTags = new Set<string>(["hd", "tv"]);
const torrentTags = new Map<number, Set<string>>([
  [0, new Set(["hd"])],
  [1, new Set(["tv"])],
]);

// Assign initial categories to some torrents
for (let i = 0; i < TOTAL_TORRENTS; i++) {
  const rand = seededRandom(i * 3571);
  const r = rand();
  if (r < 0.6) {
    // 60% have a category
    torrentCategories.set(
      i,
      MOCK_CATEGORY_NAMES[Math.floor(rand() * MOCK_CATEGORY_NAMES.length)],
    );
  }
}

const now = Date.now();
const hoursAgo = (hours: number) =>
  new Date(now - hours * 3_600_000).toISOString();

let mockRssFeeds: RssFeedConfig[] = [
  {
    name: "Open Media Weekly",
    url: "https://feeds.example.invalid/open-media.xml",
    poll_interval_secs: 900,
    category: "Media",
    enabled: true,
    auto_download: false,
  },
  {
    name: "Demo Software Releases",
    url: "https://feeds.example.invalid/software.xml",
    poll_interval_secs: 1800,
    category: "Software",
    filter_regex: "(stable|release)",
    enabled: true,
    auto_download: true,
  },
  {
    name: "Public Data Dispatch",
    url: "https://feeds.example.invalid/data.xml",
    poll_interval_secs: 3600,
    category: "Documents",
    enabled: true,
    auto_download: false,
  },
];

let mockRssItems: RssItem[] = [
  [
    "rss-01",
    "Open Media Weekly",
    "Glass River (2026) — Open Short Film 1080p",
    2.4e9,
    2,
    true,
  ],
  [
    "rss-02",
    "Demo Software Releases",
    "Amber Terminal 3.6.2 stable source bundle",
    184e6,
    5,
    true,
  ],
  [
    "rss-03",
    "Public Data Dispatch",
    "Atlas public dataset — July 2026 snapshot",
    8.7e9,
    9,
    true,
  ],
  [
    "rss-04",
    "Open Media Weekly",
    "Blue Hour — live session in lossless audio",
    1.1e9,
    15,
    false,
  ],
  [
    "rss-05",
    "Demo Software Releases",
    "CometDB 8.0.1 offline documentation",
    426e6,
    21,
    true,
  ],
  [
    "rss-06",
    "Open Media Weekly",
    "Harbor Light (2025) — open movie 2160p",
    6.8e9,
    28,
    false,
  ],
  [
    "rss-07",
    "Public Data Dispatch",
    "Transit map OpenData — 2026 Q2 export",
    4.3e9,
    37,
    true,
  ],
  [
    "rss-08",
    "Demo Software Releases",
    "Fable Engine sample projects v2.8 release",
    970e6,
    49,
    false,
  ],
  [
    "rss-09",
    "Open Media Weekly",
    "Riverside Ambience — 24-bit field recording",
    3.2e9,
    63,
    true,
  ],
  [
    "rss-10",
    "Public Data Dispatch",
    "Pocket Planetarium catalog — July update",
    740e6,
    76,
    false,
  ],
].map(([id, feedName, title, size, age, downloaded]) => ({
  id: id as string,
  feed_name: feedName as string,
  title: title as string,
  url: `magnet:?xt=urn:btih:${generateInfoHash(Number((id as string).slice(-2)))}`,
  published_at: hoursAgo(age as number),
  first_seen_at: hoursAgo((age as number) - 0.25),
  downloaded: downloaded as boolean,
  downloaded_at: downloaded ? hoursAgo((age as number) - 0.5) : null,
  category: mockRssFeeds.find((feed) => feed.name === feedName)?.category,
  size_bytes: size as number,
}));

let mockRssRules: RssRule[] = [
  {
    id: "rule-1",
    name: "Stable software",
    feed_names: ["Demo Software Releases"],
    category: "Software",
    priority: 10,
    match_regex: "(?i)(stable|release)",
    enabled: true,
  },
  {
    id: "rule-2",
    name: "Open films",
    feed_names: ["Open Media Weekly"],
    category: "Media",
    priority: 20,
    match_regex: "(?i)(open movie|short film)",
    enabled: true,
  },
];

export const MockRssAPI = {
  getFeeds: async () => [...mockRssFeeds],
  addFeed: async (feed: RssFeedConfig) => {
    mockRssFeeds = [...mockRssFeeds, feed];
  },
  updateFeed: async (name: string, feed: RssFeedConfig) => {
    mockRssFeeds = mockRssFeeds.map((entry) =>
      entry.name === name ? feed : entry,
    );
  },
  deleteFeed: async (name: string) => {
    mockRssFeeds = mockRssFeeds.filter((feed) => feed.name !== name);
  },
  getItems: async (feed?: string) =>
    mockRssItems.filter((item) => !feed || item.feed_name === feed),
  downloadItem: async (id: string) => {
    mockRssItems = mockRssItems.map((item) =>
      item.id === id
        ? { ...item, downloaded: true, downloaded_at: new Date().toISOString() }
        : item,
    );
  },
  getRules: async () => [...mockRssRules],
  addRule: async (rule: Omit<RssRule, "id">) => {
    mockRssRules = [
      ...mockRssRules,
      { ...rule, id: `rule-${mockRssRules.length + 1}` },
    ];
  },
  updateRule: async (id: string, rule: Omit<RssRule, "id">) => {
    mockRssRules = mockRssRules.map((entry) =>
      entry.id === id ? { ...rule, id } : entry,
    );
  },
  deleteRule: async (id: string) => {
    mockRssRules = mockRssRules.filter((rule) => rule.id !== id);
  },
  getSettings: async () => ({ rss_history_limit: 500 }),
};

// Mock API implementation
export const MockAPI: RtbitAPI & { getVersion: () => Promise<string> } = {
  getStreamLogsUrl: () => null,

  listTorrents: async (opts?: {
    withStats?: boolean;
  }): Promise<ListTorrentsResponse> => {
    // Simulate network delay
    await new Promise((r) => setTimeout(r, 50 + Math.random() * 100));

    const torrents: TorrentListItem[] = [];
    for (let id = 0; id < TOTAL_TORRENTS; id++) {
      if (deletedTorrents.has(id)) continue;
      torrents.push(generateTorrentListItem(id, opts?.withStats ?? false));
    }

    return { torrents, total: torrents.length };
  },

  getTorrentDetails: async (index: number): Promise<TorrentDetails> => {
    await new Promise((r) => setTimeout(r, 20 + Math.random() * 50));

    if (deletedTorrents.has(index)) {
      throw { text: "Torrent not found", status: 404 };
    }

    const totalBytes = generateTotalBytes(index);
    return {
      name: generateTorrentName(index),
      info_hash: generateInfoHash(index),
      files: generateFiles(index, totalBytes),
      total_pieces: Math.ceil(totalBytes / (256 * 1024)),
      output_folder: `/downloads/torrent_${index}`,
    };
  },

  getWebSeeds: async (index: number): Promise<WebSeedEntry[]> => {
    await new Promise((r) => setTimeout(r, 15));
    if (deletedTorrents.has(index)) {
      throw { text: "Torrent not found", status: 404 };
    }
    if (index % 3 !== 0) return [];
    return [
      { url: `https://seeds.example/t${index}/` },
      { url: `https://mirror.example/files/${index}` },
    ];
  },

  getTrackerStatus: async (index: number): Promise<TrackerStatusResponse> => {
    await new Promise((r) => setTimeout(r, 20 + Math.random() * 40));

    if (deletedTorrents.has(index)) {
      throw { text: "Torrent not found", status: 404 };
    }

    const state = torrentStates.get(index) ?? generateState(index);
    const urls = generateTrackers(index);
    const rand = seededRandom(index * 8447);

    const trackers: TrackerStatusEntry[] = urls.map((url, i) => {
      // One tracker per some torrents is erroring, for variety
      const erroring = rand() < 0.15;
      if (state !== "live") {
        return {
          url,
          state: "not_contacted",
          seeders: null,
          leechers: null,
          peers_returned: null,
          last_announce_unix: null,
          interval_secs: null,
          last_error: null,
        };
      }
      if (erroring) {
        return {
          url,
          state: "error",
          seeders: null,
          leechers: null,
          peers_returned: null,
          last_announce_unix: null,
          interval_secs: null,
          last_error: "tracker responded with 502 Bad Gateway",
        };
      }
      return {
        url,
        state: "working",
        seeders: Math.floor(rand() * 500) + 5,
        leechers: Math.floor(rand() * 200),
        peers_returned: Math.floor(rand() * 50) + 1,
        last_announce_unix:
          Math.floor(Date.now() / 1000) - Math.floor(rand() * 600) - i * 30,
        interval_secs: 1800,
        last_error: null,
      };
    });

    return { trackers };
  },

  getTorrentStats: async (index: number): Promise<TorrentStats> => {
    await new Promise((r) => setTimeout(r, 10 + Math.random() * 30));

    if (deletedTorrents.has(index)) {
      throw { text: "Torrent not found", status: 404 };
    }

    // Check for manual state override
    const override = torrentStates.get(index);
    const stats = generateTorrentStats(index);

    if (override) {
      stats.state = override;
      if (override !== "live") {
        stats.live = null;
      }
    }

    return stats;
  },

  getPeerStats: async (index: number): Promise<PeerStatsSnapshot> => {
    await new Promise((r) => setTimeout(r, 20));

    // Get stable peers and update their counters
    const peerList = getOrCreatePeers(index);
    updatePeerCounters(index);

    const peers: Record<string, any> = {};
    const rand = seededRandom(index * 4421); // For other random values

    for (const peer of peerList) {
      peers[`${peer.ip}:${peer.port}`] = {
        counters: {
          incoming_connections: Math.floor(rand() * 10),
          fetched_bytes: peer.fetchedBytes,
          uploaded_bytes: peer.uploadedBytes,
          total_time_connecting_ms: Math.floor(rand() * 10000) + 1000,
          connection_attempts: Math.floor(rand() * 3) + 1,
          connections: 1,
          errors: Math.floor(rand() * 2),
          fetched_chunks: Math.floor(peer.fetchedBytes / 16384), // ~16KB chunks
          downloaded_and_checked_pieces: Math.floor(peer.fetchedBytes / 262144), // ~256KB pieces
          total_piece_download_ms: Math.floor(rand() * 50000) + 5000,
          times_stolen_from_me: 0,
          times_i_stole: 0,
        },
        state: "live",
        conn_kind: peer.connKind,
      };
    }

    return { peers };
  },

  stats: async (): Promise<SessionStats> => {
    await new Promise((r) => setTimeout(r, 30));

    const downloadSpeed = Math.random() * 100;
    const uploadSpeed = Math.random() * 30;

    return {
      counters: {
        fetched_bytes: Math.floor(Math.random() * 100000000000),
        uploaded_bytes: Math.floor(Math.random() * 50000000000),
        blocked_incoming: Math.floor(Math.random() * 100),
        blocked_outgoing: Math.floor(Math.random() * 50),
      },
      peers: {
        queued: Math.floor(Math.random() * 500),
        connecting: Math.floor(Math.random() * 100),
        live: Math.floor(Math.random() * 300) + 50,
        seen: Math.floor(Math.random() * 2000),
        dead: Math.floor(Math.random() * 500),
        not_needed: Math.floor(Math.random() * 200),
      },
      connections: {
        tcp: {
          v4: { attempts: 1000, successes: 800, errors: 200 },
          v6: { attempts: 200, successes: 150, errors: 50 },
        },
        utp: {
          v4: { attempts: 500, successes: 300, errors: 200 },
          v6: { attempts: 100, successes: 60, errors: 40 },
        },
        socks: {
          v4: { attempts: 0, successes: 0, errors: 0 },
          v6: { attempts: 0, successes: 0, errors: 0 },
        },
      },
      download_speed: {
        mbps: downloadSpeed,
        human_readable: `${downloadSpeed.toFixed(1)} MB/s`,
      },
      upload_speed: {
        mbps: uploadSpeed,
        human_readable: `${uploadSpeed.toFixed(1)} MB/s`,
      },
      uptime_seconds: Math.floor(Date.now() / 1000) % 86400,
    };
  },

  uploadTorrent: async (): Promise<AddTorrentResponse> => {
    throw { text: "Upload not supported in mock mode", status: 501 };
  },

  updateOnlyFiles: async (): Promise<void> => {
    await new Promise((r) => setTimeout(r, 100));
  },

  pause: async (index: number): Promise<void> => {
    await new Promise((r) => setTimeout(r, 50));
    torrentStates.set(index, "paused");
  },

  start: async (index: number): Promise<void> => {
    await new Promise((r) => setTimeout(r, 50));
    torrentStates.set(index, "live");
  },

  recheck: async (index: number): Promise<void> => {
    await new Promise((r) => setTimeout(r, 50));
    torrentStates.set(index, "initializing");
  },

  addTrackers: async (): Promise<void> => {
    await new Promise((r) => setTimeout(r, 50));
  },

  forget: async (index: number): Promise<void> => {
    await new Promise((r) => setTimeout(r, 50));
    deletedTorrents.add(index);
  },

  delete: async (index: number): Promise<void> => {
    await new Promise((r) => setTimeout(r, 100));
    deletedTorrents.add(index);
  },

  getVersion: async (): Promise<string> => {
    return "mock-1.0.0";
  },

  getTorrentStreamUrl: () => null,
  getPlaylistUrl: () => null,

  getTorrentHaves: async (index: number): Promise<Uint8Array> => {
    const totalBytes = generateTotalBytes(index);
    const totalPieces = Math.ceil(totalBytes / (256 * 1024));
    const bytes = Math.ceil(totalPieces / 8);
    const haves = new Uint8Array(bytes);

    const rand = seededRandom(index * 6173);
    const progress =
      getProgressBytes(index, totalBytes, generateState(index)) / totalBytes;

    for (let i = 0; i < bytes; i++) {
      let byte = 0;
      for (let bit = 0; bit < 8; bit++) {
        if (rand() < progress) {
          byte |= 1 << (7 - bit);
        }
      }
      haves[i] = byte;
    }

    return haves;
  },

  getLimits: async (): Promise<LimitsConfig> => {
    return { upload_bps: null, download_bps: null };
  },

  setLimits: async (): Promise<void> => {
    await new Promise((r) => setTimeout(r, 50));
  },

  getDhtStats: async (): Promise<DhtStats> => {
    return {
      id: "mock-dht-node-id-abc123def456",
      outstanding_requests: 5,
      routing_table_size: 180,
      routing_table_size_v6: 24,
    };
  },

  setRustLog: async (): Promise<void> => {
    await new Promise((r) => setTimeout(r, 100));
  },

  getMetadata: async (): Promise<Uint8Array> => {
    // Return a minimal bencoded .torrent with mock trackers
    const mockTorrent =
      "d8:announce35:udp://tracker.example.com:6969/announce13:announce-listll35:udp://tracker.example.com:6969/announceel38:https://tracker2.example.org:443/announceee4:infod6:lengthi1024ee8:url-list0:e";
    return new TextEncoder().encode(mockTorrent);
  },

  getTags: async (): Promise<string[]> => {
    await new Promise((r) => setTimeout(r, 30));
    return Array.from(mockTags);
  },

  createTags: async (tags: string[]): Promise<void> => {
    await new Promise((r) => setTimeout(r, 30));
    for (const tag of tags) if (tag.trim()) mockTags.add(tag.trim());
  },

  deleteTags: async (tags: string[]): Promise<void> => {
    await new Promise((r) => setTimeout(r, 30));
    for (const tag of tags) {
      mockTags.delete(tag);
      for (const own of torrentTags.values()) own.delete(tag);
    }
  },

  renameTag: async (from: string, to: string): Promise<void> => {
    await new Promise((r) => setTimeout(r, 30));
    mockTags.delete(from);
    mockTags.add(to);
    for (const own of torrentTags.values()) {
      if (own.delete(from)) own.add(to);
    }
  },

  addTorrentTags: async (ids: number[], tags: string[]): Promise<void> => {
    await new Promise((r) => setTimeout(r, 30));
    for (const id of ids) {
      const own = torrentTags.get(id) ?? new Set<string>();
      for (const tag of tags) {
        own.add(tag);
        mockTags.add(tag);
      }
      torrentTags.set(id, own);
    }
  },

  removeTorrentTags: async (ids: number[], tags: string[]): Promise<void> => {
    await new Promise((r) => setTimeout(r, 30));
    for (const id of ids) {
      const own = torrentTags.get(id);
      if (!own) continue;
      for (const tag of tags) own.delete(tag);
    }
  },

  setTorrentTags: async (ids: number[], tags: string[]): Promise<void> => {
    await new Promise((r) => setTimeout(r, 30));
    for (const id of ids) {
      const own = new Set(tags.map((t) => t.trim()).filter(Boolean));
      torrentTags.set(id, own);
      for (const tag of own) mockTags.add(tag);
    }
  },

  getCategories: async (): Promise<Record<string, CategoryInfo>> => {
    await new Promise((r) => setTimeout(r, 30));
    return { ...mockCategories };
  },

  createCategory: async (name: string, savePath?: string): Promise<void> => {
    await new Promise((r) => setTimeout(r, 50));
    mockCategories[name] = { name, save_path: savePath };
  },

  deleteCategory: async (name: string): Promise<void> => {
    await new Promise((r) => setTimeout(r, 50));
    delete mockCategories[name];
    // Remove category from all torrents
    for (const [id, cat] of torrentCategories.entries()) {
      if (cat === name) torrentCategories.delete(id);
    }
  },

  setTorrentCategory: async (
    torrentId: number,
    category: string | null,
  ): Promise<void> => {
    await new Promise((r) => setTimeout(r, 50));
    if (category) {
      torrentCategories.set(torrentId, category);
    } else {
      torrentCategories.delete(torrentId);
    }
  },

  // Alt speed (stateful in mock so the turtle toggle works in the demo)
  getAltSpeed: async (): Promise<AltSpeedStatus> => {
    return {
      enabled: mockAltSpeed.enabled,
      config: { ...mockAltSpeed.config },
      schedule: { ...mockAltSpeed.schedule },
    };
  },
  toggleAltSpeed: async (enabled: boolean): Promise<void> => {
    await new Promise((r) => setTimeout(r, 30));
    mockAltSpeed.enabled = enabled;
  },
  setAltSpeedConfig: async (config: AltSpeedConfig): Promise<void> => {
    mockAltSpeed.config = { ...config };
  },
  getSpeedSchedule: async (): Promise<AltSpeedSchedule> => {
    return { ...mockAltSpeed.schedule };
  },
  setSpeedSchedule: async (schedule: AltSpeedSchedule): Promise<void> => {
    mockAltSpeed.schedule = { ...schedule };
  },

  // Seed limits
  getSeedLimits: async (): Promise<SeedLimitsConfig> => {
    return { ratio_limit: null, time_limit_secs: null };
  },
  setSeedLimits: async (_limits: SeedLimitsConfig): Promise<void> => {},
  getAdvanced: async (): Promise<AdvancedConfig> => {
    return {
      connect_timeout_secs: null,
      read_write_timeout_secs: null,
      keep_alive_secs: null,
    };
  },
  setAdvanced: async (_config: AdvancedConfig): Promise<void> => {},
  getProxy: async (): Promise<ProxyConfig> => ({ proxy_url: null }),
  setProxy: async (_config: ProxyConfig): Promise<void> => {},
  getIpFilter: async (): Promise<IpFilterConfig> => ({ filter: "" }),
  setIpFilter: async (_config: IpFilterConfig): Promise<void> => {},

  // Per-torrent controls
  setTorrentSeedLimits: async (
    _id: number,
    _limits: SeedLimits,
  ): Promise<void> => {},
  getTorrentLimits: async (_id: number): Promise<TorrentLimits> => {
    return {};
  },
  setTorrentLimits: async (
    _id: number,
    _limits: TorrentLimits,
  ): Promise<void> => {},
  setSequential: async (_id: number, _enabled: boolean): Promise<void> => {},
  setSuperSeed: async (_id: number, _enabled: boolean): Promise<void> => {},
  queueMoveTop: async (_id: number): Promise<void> => {},
  queueMoveBottom: async (_id: number): Promise<void> => {},
  queueMoveUp: async (_id: number): Promise<void> => {},
  queueMoveDown: async (_id: number): Promise<void> => {},

  // Folder management (mock)
  getFolders: async () => ({
    download_folder: "/downloads",
    completed_folder: null,
  }),
  setFolders: async () => {},
  getDownloadsConfig: async () => ({
    incomplete_file_suffix: false,
  }),
  setDownloadsConfig: async () => {},
  getNetworkConfig: async () => ({
    listen_ip: "::",
    listen_port: 4240,
    dht_enabled: true,
    pex_enabled: true,
    lsd_enabled: true,
    upnp_port_forward: true,
    encryption: null,
    http_api_listen: ":3030",
  }),
  browseDirectory: async (path?: string) => ({
    current: path ?? "/",
    parent:
      path && path !== "/"
        ? path.split("/").slice(0, -1).join("/") || "/"
        : null,
    entries: [
      { name: "downloads", path: (path ?? "/") + "/downloads", is_dir: true },
      { name: "completed", path: (path ?? "/") + "/completed", is_dir: true },
      { name: "media", path: (path ?? "/") + "/media", is_dir: true },
    ],
  }),
};
