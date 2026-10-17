import {
  AddTorrentResponse,
  AltSpeedConfig,
  AltSpeedSchedule,
  AltSpeedStatus,
  BrowseResponse,
  CategoryInfo,
  DhtStats,
  ErrorDetails,
  FoldersConfig,
  IndexarrIdentityStatus,
  IndexarrRecentResponse,
  IndexarrSearchResponse,
  IndexarrStatus,
  IndexarrSyncPreferences,
  LimitsConfig,
  ListTorrentsResponse,
  PeerStatsSnapshot,
  RssFeedConfig,
  RssItem,
  RssRule,
  RssSettings,
  RtbitAPI,
  SeedLimitsConfig,
  SeedLimits,
  SessionStats,
  TorrentDetails,
  TorrentLimits,
  TorrentStats,
  TrackerStatusResponse,
} from "./api-types";
import { useAuthStore } from "./stores/authStore";

// --- Auth API types ---
export interface AuthStatus {
  auth_enabled: boolean;
  setup_required: boolean;
}

export interface TokenResponse {
  access_token: string;
  refresh_token: string;
  token_type: string;
  expires_in: number;
}

// Define API URL and base path
const apiUrl = (() => {
  if (window.origin === "null") {
    return "http://localhost:3030";
  }

  const url = new URL(window.location.href);

  // Vite's development server runs on 3031 and forwards API requests to a
  // separately running backend on 3030. The production web UI is served by
  // rtbit itself under /web/ on the API port, so it must use same-origin API
  // requests. The desktop dev shell on 1420 also talks to the backend on 3030.
  if (url.port == "3031" && (url.pathname === "/" || url.pathname === "")) {
    return `${url.protocol}//${url.hostname}:3030`;
  }
  if (url.port == "1420") {
    return `${url.protocol}//${url.hostname}:3030`;
  }

  // Remove "/web" or "/web/" from the end and also ending slash.
  const path = /(.*?)\/?(\/web\/?)?$/.exec(url.pathname)![1] ?? "";
  return path;
})();

// Get auth headers if token is available
const getAuthHeaders = (): Record<string, string> => {
  const token = useAuthStore.getState().getAccessToken();
  if (token) {
    return { Authorization: `Bearer ${token}` };
  }
  return {};
};

// Try to refresh the token, returns true if successful
const tryRefreshToken = async (): Promise<boolean> => {
  const { refreshToken, setTokens, expireSession } = useAuthStore.getState();
  if (!refreshToken) return false;

  try {
    const url = apiUrl + "/auth/refresh";
    const response = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ refresh_token: refreshToken }),
    });
    if (!response.ok) {
      expireSession();
      return false;
    }
    const tokens: TokenResponse = await response.json();
    setTokens(tokens.access_token, tokens.refresh_token, tokens.expires_in);
    return true;
  } catch {
    expireSession();
    return false;
  }
};

const makeBinaryRequest = async (path: string): Promise<ArrayBuffer> => {
  const url = apiUrl + path;
  const response = await fetch(url, {
    method: "GET",
    headers: {
      Accept: "application/octet-stream",
      ...getAuthHeaders(),
    },
  });

  if (response.status === 401) {
    // Try refresh
    if (await tryRefreshToken()) {
      const retry = await fetch(url, {
        method: "GET",
        headers: {
          Accept: "application/octet-stream",
          ...getAuthHeaders(),
        },
      });
      if (retry.ok) return retry.arrayBuffer();
    }
    useAuthStore.getState().expireSession();
    throw new Error("Authentication required");
  }

  if (!response.ok) {
    throw new Error(`HTTP ${response.status}: ${response.statusText}`);
  }

  return response.arrayBuffer();
};

const makeRequest = async (
  method: string,
  path: string,
  data?: any,
  isJson?: boolean,
): Promise<any> => {
  console.log(method, path);
  const url = apiUrl + path;
  const authHeaders = getAuthHeaders();
  const options: RequestInit = {
    method,
    headers: {
      Accept: "application/json",
      ...authHeaders,
    },
  };
  if (isJson) {
    options.headers = {
      Accept: "application/json",
      "Content-Type": "application/json",
      ...authHeaders,
    };
    options.body = JSON.stringify(data);
  } else {
    options.body = data;
  }

  const error: ErrorDetails = {
    method: method,
    path: path,
    text: "",
  };

  let response: Response;

  try {
    response = await fetch(url, options);
  } catch (e) {
    error.text = "network error";
    return Promise.reject(error);
  }

  // Handle 401 — try token refresh
  if (response.status === 401) {
    if (await tryRefreshToken()) {
      // Retry with new token
      const retryHeaders = getAuthHeaders();
      const retryOptions: RequestInit = {
        ...options,
        headers: isJson
          ? {
              Accept: "application/json",
              "Content-Type": "application/json",
              ...retryHeaders,
            }
          : { Accept: "application/json", ...retryHeaders },
      };
      try {
        response = await fetch(url, retryOptions);
      } catch (e) {
        error.text = "network error";
        return Promise.reject(error);
      }
      if (response.ok) {
        return response.json();
      }
    }
    // Refresh failed or retry failed
    useAuthStore.getState().expireSession();
    error.status = 401;
    error.statusText = "401 Unauthorized";
    error.text = "Session expired. Please log in again.";
    return Promise.reject(error);
  }

  error.status = response.status;
  error.statusText = `${response.status} ${response.statusText}`;

  if (!response.ok) {
    const errorBody = await response.text();
    try {
      const json = JSON.parse(errorBody);
      error.text =
        json.human_readable !== undefined
          ? json.human_readable
          : JSON.stringify(json, null, 2);
    } catch (e) {
      error.text = errorBody;
    }
    return Promise.reject(error);
  }
  const result = await response.json();
  return result;
};

// --- Auth API (no auth headers needed for these) ---
export const AuthAPI = {
  getStatus: async (): Promise<AuthStatus> => {
    const url = apiUrl + "/auth/status";
    const response = await fetch(url);
    if (!response.ok) {
      // If /auth/status returns 404, auth is not available (old server)
      return { auth_enabled: false, setup_required: false };
    }
    return response.json();
  },

  login: async (username: string, password: string): Promise<TokenResponse> => {
    const url = apiUrl + "/auth/login";
    const response = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ username, password }),
    });
    if (!response.ok) {
      const text = await response.text();
      throw new Error(text || "Login failed");
    }
    return response.json();
  },

  setup: async (
    username: string,
    password: string,
    setupToken: string,
  ): Promise<TokenResponse> => {
    const url = apiUrl + "/auth/setup";
    const response = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        username,
        password,
        setup_token: setupToken,
      }),
    });
    if (!response.ok) {
      const text = await response.text();
      throw new Error(text || "Setup failed");
    }
    return response.json();
  },

  logout: async (refreshToken: string): Promise<void> => {
    const url = apiUrl + "/auth/logout";
    await fetch(url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...getAuthHeaders(),
      },
      body: JSON.stringify({ refresh_token: refreshToken }),
    });
  },

  changeCredentials: async (
    currentPassword: string,
    newUsername?: string,
    newPassword?: string,
  ): Promise<void> => {
    const url = apiUrl + "/auth/change_credentials";
    const response = await fetch(url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...getAuthHeaders(),
      },
      body: JSON.stringify({
        current_password: currentPassword,
        new_username: newUsername || undefined,
        new_password: newPassword || undefined,
      }),
    });
    if (!response.ok) {
      const text = await response.text();
      throw new Error(text || "Failed to change credentials");
    }
  },
};

export const API: RtbitAPI & { getVersion: () => Promise<string> } = {
  getStreamLogsUrl: () => apiUrl + "/stream_logs",
  listTorrents: (opts?: {
    withStats?: boolean;
  }): Promise<ListTorrentsResponse> => {
    const url = opts?.withStats ? "/torrents?with_stats=true" : "/torrents";
    return makeRequest("GET", url);
  },
  getTorrentDetails: (index: number): Promise<TorrentDetails> => {
    return makeRequest("GET", `/torrents/${index}`);
  },
  getTorrentStats: (index: number): Promise<TorrentStats> => {
    return makeRequest("GET", `/torrents/${index}/stats/v1`);
  },
  getTrackerStatus: (index: number): Promise<TrackerStatusResponse> => {
    return makeRequest("GET", `/torrents/${index}/trackers`);
  },
  getPeerStats: (index: number): Promise<PeerStatsSnapshot> => {
    return makeRequest("GET", `/torrents/${index}/peer_stats?state=live`);
  },
  stats: (): Promise<SessionStats> => {
    return makeRequest("GET", "/stats");
  },

  uploadTorrent: (data, opts): Promise<AddTorrentResponse> => {
    let url = "/torrents?&overwrite=true";
    if (opts?.list_only) {
      url += "&list_only=true";
    }
    if (opts?.only_files != null) {
      url += `&only_files=${opts.only_files.join(",")}`;
    }
    if (opts?.peer_opts?.connect_timeout) {
      url += `&peer_connect_timeout=${opts.peer_opts.connect_timeout}`;
    }
    if (opts?.peer_opts?.read_write_timeout) {
      url += `&peer_read_write_timeout=${opts.peer_opts.read_write_timeout}`;
    }
    if (opts?.paused) {
      url += "&paused=true";
    }
    if (opts?.initial_peers) {
      url += `&initial_peers=${opts.initial_peers.join(",")}`;
    }
    if (opts?.output_folder) {
      url += `&output_folder=${opts.output_folder}`;
    }
    if (opts?.category) {
      url += `&category=${encodeURIComponent(opts.category)}`;
    }
    if (typeof data === "string") {
      url += "&is_url=true";
    }
    return makeRequest("POST", url, data);
  },

  updateOnlyFiles: (index: number, files: number[]): Promise<void> => {
    const url = `/torrents/${index}/update_only_files`;
    return makeRequest(
      "POST",
      url,
      {
        only_files: files,
      },
      true,
    );
  },

  pause: (index: number): Promise<void> => {
    return makeRequest("POST", `/torrents/${index}/pause`);
  },

  start: (index: number): Promise<void> => {
    return makeRequest("POST", `/torrents/${index}/start`);
  },

  recheck: (index: number): Promise<void> => {
    return makeRequest("POST", `/torrents/${index}/recheck`);
  },

  addTrackers: (index: number, trackers: string[]): Promise<void> => {
    return makeRequest(
      "POST",
      `/torrents/${index}/trackers`,
      { trackers },
      true,
    );
  },

  forget: (index: number): Promise<void> => {
    return makeRequest("POST", `/torrents/${index}/forget`);
  },

  delete: (index: number): Promise<void> => {
    return makeRequest("POST", `/torrents/${index}/delete`);
  },
  getVersion: async (): Promise<string> => {
    const r = await makeRequest("GET", "/");
    return r.version;
  },
  getTorrentStreamUrl: (
    index: number,
    file_id: number,
    filename?: string | null,
  ) => {
    let url = apiUrl + `/torrents/${index}/stream/${file_id}`;
    if (filename) {
      url += `/${filename}`;
    }
    return url;
  },
  getPlaylistUrl: (index: number) => {
    return (apiUrl || window.origin) + `/torrents/${index}/playlist`;
  },
  getTorrentHaves: async (index: number): Promise<Uint8Array> => {
    return new Uint8Array(await makeBinaryRequest(`/torrents/${index}/haves`));
  },
  getLimits: (): Promise<LimitsConfig> => {
    return makeRequest("GET", "/torrents/limits");
  },
  setLimits: (limits: LimitsConfig): Promise<void> => {
    return makeRequest("POST", "/torrents/limits", limits, true);
  },
  getDhtStats: (): Promise<DhtStats> => {
    return makeRequest("GET", "/dht/stats");
  },
  setRustLog: (value: string): Promise<void> => {
    return makeRequest("POST", "/rust_log", value);
  },
  getMetadata: async (index: number): Promise<Uint8Array> => {
    return new Uint8Array(
      await makeBinaryRequest(`/torrents/${index}/metadata`),
    );
  },
  getCategories: (): Promise<Record<string, CategoryInfo>> => {
    return makeRequest("GET", "/torrents/categories");
  },
  createCategory: (name: string, savePath?: string): Promise<void> => {
    return makeRequest(
      "POST",
      "/torrents/categories",
      { name, save_path: savePath },
      true,
    );
  },
  deleteCategory: (name: string): Promise<void> => {
    return makeRequest(
      "DELETE",
      `/torrents/categories/${encodeURIComponent(name)}`,
    );
  },
  setTorrentCategory: (
    torrentId: number,
    category: string | null,
  ): Promise<void> => {
    return makeRequest(
      "POST",
      `/torrents/${torrentId}/set_category`,
      { category },
      true,
    );
  },

  // Alt speed
  getAltSpeed: (): Promise<AltSpeedStatus> => {
    return makeRequest("GET", "/speed/alt");
  },
  toggleAltSpeed: (enabled: boolean): Promise<void> => {
    return makeRequest("POST", "/speed/alt", { enabled }, true);
  },
  setAltSpeedConfig: (config: AltSpeedConfig): Promise<void> => {
    return makeRequest("POST", "/speed/alt/config", config, true);
  },
  getSpeedSchedule: (): Promise<AltSpeedSchedule> => {
    return makeRequest("GET", "/speed/schedule");
  },
  setSpeedSchedule: (schedule: AltSpeedSchedule): Promise<void> => {
    return makeRequest("POST", "/speed/schedule", schedule, true);
  },

  // Seed limits
  getSeedLimits: (): Promise<SeedLimitsConfig> => {
    return makeRequest("GET", "/torrents/seed_limits");
  },
  setSeedLimits: (limits: SeedLimitsConfig): Promise<void> => {
    return makeRequest("POST", "/torrents/seed_limits", limits, true);
  },

  // Per-torrent controls
  setTorrentSeedLimits: (id: number, limits: SeedLimits): Promise<void> => {
    return makeRequest("POST", `/torrents/${id}/seed_limits`, limits, true);
  },
  getTorrentLimits: (id: number): Promise<TorrentLimits> => {
    return makeRequest("GET", `/torrents/${id}/limits`);
  },
  setTorrentLimits: (id: number, limits: TorrentLimits): Promise<void> => {
    return makeRequest("POST", `/torrents/${id}/limits`, limits, true);
  },
  setSequential: (id: number, enabled: boolean): Promise<void> => {
    return makeRequest("POST", `/torrents/${id}/sequential`, { enabled }, true);
  },
  setSuperSeed: (id: number, enabled: boolean): Promise<void> => {
    return makeRequest("POST", `/torrents/${id}/super_seed`, { enabled }, true);
  },
  queueMoveTop: (id: number): Promise<void> => {
    return makeRequest("POST", `/torrents/${id}/queue/top`);
  },
  queueMoveBottom: (id: number): Promise<void> => {
    return makeRequest("POST", `/torrents/${id}/queue/bottom`);
  },
  queueMoveUp: (id: number): Promise<void> => {
    return makeRequest("POST", `/torrents/${id}/queue/up`);
  },
  queueMoveDown: (id: number): Promise<void> => {
    return makeRequest("POST", `/torrents/${id}/queue/down`);
  },

  // Folder management
  getFolders: (): Promise<FoldersConfig> => {
    return makeRequest("GET", "/session/folders");
  },
  setFolders: (config: FoldersConfig): Promise<void> => {
    return makeRequest("POST", "/session/folders", config, true);
  },
  browseDirectory: (path?: string): Promise<BrowseResponse> => {
    const params = new URLSearchParams();
    if (path) params.set("path", path);
    return makeRequest("GET", `/fs/browse?${params.toString()}`);
  },
};

// --- Indexarr API ---

export const IndexarrAPI = {
  getStatus: (): Promise<IndexarrStatus> => {
    return makeRequest("GET", "/indexarr/status");
  },

  search: (
    query: string,
    filters?: Record<string, string>,
    limit = 50,
    offset = 0,
  ): Promise<IndexarrSearchResponse> => {
    const params = new URLSearchParams();
    params.set("q", query);
    params.set("limit", String(limit));
    params.set("offset", String(offset));
    if (filters) {
      for (const [key, value] of Object.entries(filters)) {
        if (value) params.set(key, value);
      }
    }
    return makeRequest("GET", `/indexarr/search?${params.toString()}`);
  },

  getRecent: (limit = 50): Promise<IndexarrRecentResponse> => {
    return makeRequest("GET", `/indexarr/recent?limit=${limit}`);
  },

  getTrending: (limit = 50): Promise<IndexarrSearchResponse> => {
    return makeRequest("GET", `/indexarr/trending?limit=${limit}`);
  },

  getIdentityStatus: (): Promise<IndexarrIdentityStatus> => {
    return makeRequest("GET", "/indexarr/identity/status");
  },

  acknowledgeIdentity: (): Promise<void> => {
    return makeRequest("POST", "/indexarr/identity/acknowledge");
  },

  getSyncPreferences: (): Promise<IndexarrSyncPreferences> => {
    return makeRequest("GET", "/indexarr/sync/preferences");
  },

  setSyncPreferences: (prefs: {
    import_categories: string[];
    sync_comments: boolean;
  }): Promise<IndexarrSyncPreferences> => {
    return makeRequest("POST", "/indexarr/sync/preferences", prefs, true);
  },
};

// --- RSS API ---

export const RssAPI = {
  // Feed config management
  getFeeds: (): Promise<RssFeedConfig[]> => {
    return makeRequest("GET", "/rss/feeds");
  },

  addFeed: (feed: RssFeedConfig): Promise<void> => {
    return makeRequest("POST", "/rss/feeds", feed, true);
  },

  updateFeed: (name: string, feed: RssFeedConfig): Promise<void> => {
    return makeRequest(
      "PUT",
      `/rss/feeds/${encodeURIComponent(name)}`,
      feed,
      true,
    );
  },

  deleteFeed: (name: string): Promise<void> => {
    return makeRequest("DELETE", `/rss/feeds/${encodeURIComponent(name)}`);
  },

  // Feed items
  getItems: (feed?: string, limit = 500): Promise<RssItem[]> => {
    const params = new URLSearchParams();
    if (feed) params.set("feed", feed);
    params.set("limit", String(limit));
    return makeRequest("GET", `/rss/items?${params.toString()}`);
  },

  downloadItem: (id: string): Promise<void> => {
    return makeRequest("POST", `/rss/items/${encodeURIComponent(id)}/download`);
  },

  // Download rules
  getRules: (): Promise<RssRule[]> => {
    return makeRequest("GET", "/rss/rules");
  },

  addRule: (rule: {
    name: string;
    feed_names: string[];
    category?: string | null;
    priority?: number;
    match_regex: string;
    enabled?: boolean;
  }): Promise<void> => {
    return makeRequest("POST", "/rss/rules", rule, true);
  },

  updateRule: (
    id: string,
    rule: {
      name: string;
      feed_names: string[];
      category?: string | null;
      priority?: number;
      match_regex: string;
      enabled?: boolean;
    },
  ): Promise<void> => {
    return makeRequest(
      "PUT",
      `/rss/rules/${encodeURIComponent(id)}`,
      rule,
      true,
    );
  },

  deleteRule: (id: string): Promise<void> => {
    return makeRequest("DELETE", `/rss/rules/${encodeURIComponent(id)}`);
  },

  // Settings
  getSettings: (): Promise<RssSettings> => {
    return makeRequest("GET", "/rss/settings");
  },
};
