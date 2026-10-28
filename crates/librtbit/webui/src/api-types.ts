// Interface for the Torrent API response
export interface TorrentId {
  id: number;
  info_hash: string;
}

export interface TorrentFile {
  name: string;
  components: string[];
  length: number;
  included: boolean;
  attributes: TorrentFileAttributes;
}

export interface TorrentFileAttributes {
  symlink: boolean;
  hidden: boolean;
  padding: boolean;
  executable: boolean;
}

// Interface for the Torrent Details API response (with files, from individual endpoint)
export interface TorrentDetails {
  name: string | null;
  info_hash: string;
  files: Array<TorrentFile>;
  total_pieces?: number;
  output_folder: string;
  category?: string;
}

// Interface for torrent list item (from bulk /torrents?with_stats=true endpoint)
// This matches TorrentDetailsResponse from the backend, but files are not included in the list
export interface TorrentListItem {
  id: number;
  info_hash: string;
  name: string | null;
  output_folder: string;
  total_pieces: number;
  stats?: TorrentStats;
  category?: string;
  /** Tags assigned to this torrent. */
  tags?: string[];
  /** Unix timestamp (seconds) when the torrent was added. */
  added_on?: number;
  /** Announce URLs configured for this torrent. */
  trackers?: string[];
}

export interface AddTorrentResponse {
  id: number | null;
  details: TorrentDetails;
  output_folder: string;
  seen_peers?: Array<string>;
}

export interface ListTorrentsResponse {
  torrents: Array<TorrentListItem>;
  total: number;
}

export interface Speed {
  mbps: number;
  human_readable: string;
}

export interface AggregatePeerStats {
  queued: number;
  connecting: number;
  live: number;
  seen: number;
  dead: number;
  not_needed: number;
}

export type ConnectionKind = "tcp" | "utp" | "socks";

export interface PeerCounters {
  incoming_connections: number;
  fetched_bytes: number;
  uploaded_bytes: number;
  total_time_connecting_ms: number;
  connection_attempts: number;
  connections: number;
  errors: number;
  fetched_chunks: number;
  downloaded_and_checked_pieces: number;
  total_piece_download_ms: number;
  times_stolen_from_me: number;
  times_i_stole: number;
}

export interface PeerStats {
  counters: PeerCounters;
  state: string;
  conn_kind: ConnectionKind | null;
}

export interface PeerStatsSnapshot {
  peers: Record<string, PeerStats>;
}

export interface ConnectionStatSingle {
  attempts: number;
  successes: number;
  errors: number;
}
export interface ConnectionStatsPerFamily {
  v4: ConnectionStatSingle;
  v6: ConnectionStatSingle;
}
export interface ConnectionStats {
  tcp: ConnectionStatsPerFamily;
  utp: ConnectionStatsPerFamily;
  socks: ConnectionStatsPerFamily;
}

export interface SessionCounters {
  fetched_bytes: number;
  uploaded_bytes: number;
  blocked_incoming: number;
  blocked_outgoing: number;
}

export interface SessionStats {
  counters: SessionCounters;
  peers: AggregatePeerStats;
  connections: ConnectionStats;
  download_speed: Speed;
  upload_speed: Speed;
  uptime_seconds: number;
}

export interface LimitsConfig {
  upload_bps?: number | null;
  download_bps?: number | null;
  peer_limit?: number | null;
  concurrent_init_limit?: number | null;
  max_active_downloads?: number | null;
  max_active_uploads?: number | null;
  max_active_total?: number | null;
}

// Per-tracker announce status
export type TrackerAnnounceState =
  | "not_contacted"
  | "updating"
  | "working"
  | "error"
  | "disabled";

export interface TrackerStatusEntry {
  url: string;
  state: TrackerAnnounceState;
  seeders: number | null;
  leechers: number | null;
  peers_returned: number | null;
  last_announce_unix: number | null;
  interval_secs: number | null;
  last_error: string | null;
}

export interface WebSeedEntry {
  url: string;
}

export interface TrackerStatusResponse {
  trackers: TrackerStatusEntry[];
}

// Alt speed config
export interface AltSpeedConfig {
  download_rate: number | null;
  upload_rate: number | null;
}

export interface AltSpeedSchedule {
  enabled: boolean;
  start_minutes: number; // minutes from midnight
  end_minutes: number;
  days: number; // bitmask: 1=Mon,2=Tue,4=Wed,8=Thu,16=Fri,32=Sat,64=Sun
}

export interface AltSpeedStatus {
  enabled: boolean;
  config: AltSpeedConfig;
  schedule: AltSpeedSchedule | null;
}

// Seed limits
export interface SeedLimitsConfig {
  ratio_limit: number | null;
  time_limit_secs: number | null;
}

// Queue limits
export interface QueueLimitsConfig {
  max_active_downloads: number;
  max_active_uploads: number;
  max_active_total: number;
}

export interface DhtStats {
  id?: string;
  outstanding_requests?: number;
  routing_table_size?: number;
  routing_table_size_v6?: number;
  // Older mocks reported peer counts instead of the routing table.
  seen_peers?: number;
  have_peers?: number;
  inflight_peers?: number;
  [key: string]: string | number | undefined;
}

// Interface for the Torrent Stats API response
export interface LiveTorrentStats {
  snapshot: {
    have_bytes: number;
    downloaded_and_checked_bytes: number;
    downloaded_and_checked_pieces: number;
    fetched_bytes: number;
    uploaded_bytes: number;
    initially_needed_bytes: number;
    remaining_bytes: number;
    total_bytes: number;
    total_piece_download_ms: number;
    peer_stats: AggregatePeerStats;
  };
  average_piece_download_time: {
    secs: number;
    nanos: number;
  };
  download_speed: Speed;
  upload_speed: Speed;
  all_time_download_speed: {
    mbps: number;
    human_readable: string;
  };
  time_remaining: {
    human_readable: string;
    duration?: {
      secs: number;
    };
  } | null;
}

export const STATE_INITIALIZING = "initializing";
export const STATE_PAUSED = "paused";
export const STATE_LIVE = "live";
export const STATE_ERROR = "error";

export interface TorrentStats {
  state: "initializing" | "paused" | "live" | "error";
  error: string | null;
  file_progress: number[];
  progress_bytes: number;
  finished: boolean;
  total_bytes: number;
  live: LiveTorrentStats | null;
  ratio?: number;
  seeding_time_secs?: number;
  seed_ratio_limit?: number | null;
  seed_time_limit_secs?: number | null;
  super_seeding?: boolean;
  sequential?: boolean;
  min_piece_availability?: number;
  avg_piece_availability?: number;
  queue_state?: "Active" | "Queued" | "ManuallyPaused";
  queue_position?: number;
}

export interface TorrentLimits {
  download_rate?: number;
  upload_rate?: number;
}

export interface SeedLimits {
  ratio_limit?: number | null;
  time_limit_secs?: number | null;
}

export interface ErrorDetails {
  id?: number;
  method?: string;
  path?: string;
  status?: number;
  statusText?: string;
  text: string | React.ReactNode;
}

export type Duration = number;

export interface PeerConnectionOptions {
  connect_timeout?: Duration | null;
  read_write_timeout?: Duration | null;
  keep_alive_interval?: Duration | null;
}

export interface AddTorrentOptions {
  paused?: boolean;
  only_files_regex?: string | null;
  only_files?: number[] | null;
  overwrite?: boolean;
  list_only?: boolean;
  output_folder?: string | null;
  sub_folder?: string | null;
  peer_opts?: PeerConnectionOptions | null;
  force_tracker_interval?: Duration | null;
  initial_peers?: string[] | null; // Assuming SocketAddr is equivalent to a string in TypeScript
  preferred_id?: number | null;
  category?: string;
}

export interface CategoryInfo {
  name: string;
  save_path?: string;
}

export type Value = string | number | boolean;

export interface Span {
  name: string;
  [key: string]: Value;
}

/*
Example log line

const EXAMPLE_LOG_JSON: JSONLogLine = {
  timestamp: "2023-12-08T21:48:13.649165Z",
  level: "DEBUG",
  fields: { message: "successfully port forwarded 192.168.0.112:4225" },
  target: "librtbit_upnp",
  span: { port: 4225, name: "manage_port" },
  spans: [
    { port: 4225, name: "upnp_forward" },
    {
      location: "http://192.168.0.1:49152/IGDdevicedesc_brlan0.xml",
      name: "upnp_endpoint",
    },
    { device: "ARRIS TG3492LG", name: "device" },
    { device: "WANDevice:1", name: "device" },
    { device: "WANConnectionDevice:1", name: "device" },
    { url: "/upnp/control/WANIPConnection0", name: "service" },
    { port: 4225, name: "manage_port" },
  ],
};
*/
export interface JSONLogLine {
  level: string;
  timestamp: string;
  fields: {
    message: string;
    [key: string]: Value;
  };
  target: string;
  span: Span;
  spans: Span[];
}

// --- Indexarr types ---

export interface IndexarrStatus {
  enabled: boolean;
  reachable?: boolean;
  error?: string;
  indexarr?: {
    status: string;
    version: string;
    ready: boolean;
  };
}

export interface IndexarrSearchResult {
  info_hash: string;
  name: string | null;
  size: number | null;
  content_type: string | null;
  resolution: string | null;
  codec: string | null;
  video_source: string | null;
  year: number | null;
  season: number | null;
  episode: number | null;
  seed_count: number;
  peer_count: number;
  discovered_at: string | null;
  resolved_at: string | null;
  tags: string[];
  audio_codec: string | null;
  hdr: string | null;
  platform: string | null;
  quality_score: number | null;
  network: string | null;
  edition: string | null;
  trackers: string[] | null;
}

export interface IndexarrSearchResponse {
  results: IndexarrSearchResult[];
  total: number;
  offset: number;
  limit: number;
  facets?: Record<string, { value: string; count: number }[]>;
}

export interface IndexarrRecentItem {
  info_hash: string;
  name: string | null;
  size: number | null;
  content_type: string | null;
  resolution: string | null;
  seed_count: number;
  peer_count: number;
  resolved_at: string | null;
  source: string | null;
  trackers: string[] | null;
}

export interface IndexarrRecentResponse {
  results: IndexarrRecentItem[];
}

export interface IndexarrIdentityStatus {
  initialized: boolean;
  needs_onboarding: boolean;
  contributor_id?: string;
  recovery_key?: string | null;
  public_key?: string;
}

export interface IndexarrSyncPreferences {
  import_categories: string[];
  all_categories: string[];
  sync_comments: boolean;
}

// --- RSS types ---

export interface RssFeedConfig {
  name: string;
  url: string;
  poll_interval_secs: number;
  category?: string | null;
  filter_regex?: string | null;
  enabled: boolean;
  auto_download: boolean;
}

export interface RssItem {
  id: string;
  feed_name: string;
  title: string;
  url?: string | null;
  published_at?: string | null;
  first_seen_at: string;
  downloaded: boolean;
  downloaded_at?: string | null;
  category?: string | null;
  size_bytes: number;
}

export interface RssRule {
  id: string;
  name: string;
  feed_names: string[];
  category?: string | null;
  priority: number;
  match_regex: string;
  enabled: boolean;
}

export interface RssSettings {
  rss_history_limit: number;
}

// Folder configuration
export interface DownloadsConfig {
  incomplete_file_suffix: boolean;
}

export interface FoldersConfig {
  download_folder: string;
  completed_folder: string | null;
}

// Session-wide peer connection timeouts, in seconds. null means the engine default.
export interface AdvancedConfig {
  connect_timeout_secs: number | null;
  read_write_timeout_secs: number | null;
  keep_alive_secs: number | null;
}

/** Startup-only peer and discovery settings. `encryption` is null: unsupported. */

export interface ProxyConfig {
  proxy_url: string | null;
}

export interface IpFilterConfig {
  filter: string;
}


export interface NetworkConfig {
  listen_ip: string | null;
  listen_port: number | null;
  dht_enabled: boolean;
  pex_enabled: boolean;
  lsd_enabled: boolean;
  upnp_port_forward: boolean;
  encryption: string | null;
  http_api_listen: string;
}

// Directory browsing
export interface DirEntry {
  name: string;
  path: string;
  is_dir: boolean;
}

export interface BrowseResponse {
  current: string;
  parent: string | null;
  entries: DirEntry[];
}

export interface RtbitAPI {
  getPlaylistUrl: (index: number) => string | null;
  getStreamLogsUrl: () => string | null;
  listTorrents: (opts?: {
    withStats?: boolean;
  }) => Promise<ListTorrentsResponse>;
  getTorrentDetails: (index: number) => Promise<TorrentDetails>;
  getTorrentStats: (index: number) => Promise<TorrentStats>;
  getTrackerStatus: (index: number) => Promise<TrackerStatusResponse>;
  getWebSeeds: (index: number) => Promise<WebSeedEntry[]>;
  getTorrentHaves: (index: number) => Promise<Uint8Array>;
  getPeerStats: (index: number) => Promise<PeerStatsSnapshot>;
  getTorrentStreamUrl: (
    index: number,
    file_id: number,
    filename?: string | null,
  ) => string | null;
  uploadTorrent: (
    data: string | File,
    opts?: AddTorrentOptions,
  ) => Promise<AddTorrentResponse>;

  pause: (index: number) => Promise<void>;
  updateOnlyFiles: (index: number, files: number[]) => Promise<void>;
  start: (index: number) => Promise<void>;
  recheck: (index: number) => Promise<void>;
  addTrackers: (index: number, trackers: string[]) => Promise<void>;
  forget: (index: number) => Promise<void>;
  delete: (index: number) => Promise<void>;
  stats: () => Promise<SessionStats>;
  getLimits: () => Promise<LimitsConfig>;
  setLimits: (limits: LimitsConfig) => Promise<void>;
  getDhtStats: () => Promise<DhtStats>;
  setRustLog: (value: string) => Promise<void>;
  getMetadata: (index: number) => Promise<Uint8Array>;

  // Category management
  getCategories: () => Promise<Record<string, CategoryInfo>>;
  createCategory: (name: string, savePath?: string) => Promise<void>;
  deleteCategory: (name: string) => Promise<void>;
  setTorrentCategory: (
    torrentId: number,
    category: string | null,
  ) => Promise<void>;

  // Tags
  getTags: () => Promise<string[]>;
  createTags: (tags: string[]) => Promise<void>;
  deleteTags: (tags: string[]) => Promise<void>;
  addTorrentTags: (ids: number[], tags: string[]) => Promise<void>;
  removeTorrentTags: (ids: number[], tags: string[]) => Promise<void>;

  // Alt speed
  getAltSpeed: () => Promise<AltSpeedStatus>;
  toggleAltSpeed: (enabled: boolean) => Promise<void>;
  setAltSpeedConfig: (config: AltSpeedConfig) => Promise<void>;
  getSpeedSchedule: () => Promise<AltSpeedSchedule>;
  setSpeedSchedule: (schedule: AltSpeedSchedule) => Promise<void>;

  // Seed limits
  getSeedLimits: () => Promise<SeedLimitsConfig>;
  getAdvanced: () => Promise<AdvancedConfig>;
  setAdvanced: (config: AdvancedConfig) => Promise<void>;
  getProxy: () => Promise<ProxyConfig>;
  setProxy: (config: ProxyConfig) => Promise<void>;
  getIpFilter: () => Promise<IpFilterConfig>;
  setIpFilter: (config: IpFilterConfig) => Promise<void>;
  setSeedLimits: (limits: SeedLimitsConfig) => Promise<void>;

  // Per-torrent controls
  setTorrentSeedLimits: (id: number, limits: SeedLimits) => Promise<void>;
  getTorrentLimits: (id: number) => Promise<TorrentLimits>;
  setTorrentLimits: (id: number, limits: TorrentLimits) => Promise<void>;
  setSequential: (id: number, enabled: boolean) => Promise<void>;
  setSuperSeed: (id: number, enabled: boolean) => Promise<void>;
  queueMoveTop: (id: number) => Promise<void>;
  queueMoveBottom: (id: number) => Promise<void>;
  queueMoveUp: (id: number) => Promise<void>;
  queueMoveDown: (id: number) => Promise<void>;

  // Folder management
  getFolders: () => Promise<FoldersConfig>;
  setFolders: (config: FoldersConfig) => Promise<void>;
  getDownloadsConfig: () => Promise<DownloadsConfig>;
  setDownloadsConfig: (config: DownloadsConfig) => Promise<void>;
  getNetworkConfig: () => Promise<NetworkConfig>;
  browseDirectory: (path?: string) => Promise<BrowseResponse>;
}
