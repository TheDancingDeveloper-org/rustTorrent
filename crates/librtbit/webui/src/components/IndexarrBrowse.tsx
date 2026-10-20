import { useCallback, useContext, useEffect, useRef, useState } from "react";
import {
  FaDownload,
  FaSearch,
  FaSeedling,
  FaUsers,
  FaCog,
} from "react-icons/fa";
import { GoX } from "react-icons/go";
import debounce from "lodash.debounce";

import { APIContext } from "../context";
import {
  AddTorrentResponse,
  ErrorDetails,
  IndexarrSearchResult,
  IndexarrRecentItem,
} from "../api-types";
import { useIndexarrStore } from "../stores/indexarrStore";
import { useErrorStore } from "../stores/errorStore";
import { formatBytes } from "../helper/formatBytes";
import { IndexarrAPI } from "../http-api";
import { Spinner } from "./Spinner";
import { IndexarrSetup } from "./IndexarrSetup";
import { FileSelectionModal } from "./modal/FileSelectionModal";
import { ErrorWithLabel } from "../rtbit-web";

const DEFAULT_TRACKERS = [
  "udp://tracker.opentrackr.org:1337/announce",
  "udp://open.stealth.si:80/announce",
];

function buildMagnet(
  infoHash: string,
  name: string | null,
  trackers: string[] | null,
): string {
  const dn = name ? `&dn=${encodeURIComponent(name)}` : "";
  const trList = trackers && trackers.length > 0 ? trackers : DEFAULT_TRACKERS;
  const tr = trList.map((t) => `&tr=${encodeURIComponent(t)}`).join("");
  return `magnet:?xt=urn:btih:${infoHash}${dn}${tr}`;
}

function contentTypeBadge(ct: string | null) {
  if (!ct) return null;
  // Categorical hues with no status meaning, so raw palette steps are used:
  // the 800 step on light surfaces and the 300 step on dark both clear 4.5:1
  // (guarded by helper/featurePageTokens.test.mjs).
  const colors: Record<string, string> = {
    movie: "bg-blue-500/15 text-blue-800 dark:text-blue-300",
    tv_show: "bg-purple-500/15 text-purple-800 dark:text-purple-300",
    music: "bg-green-500/15 text-green-800 dark:text-green-300",
    ebook: "bg-yellow-500/15 text-amber-800 dark:text-yellow-300",
    game: "bg-red-500/15 text-red-800 dark:text-red-300",
    software: "bg-cyan-500/15 text-cyan-800 dark:text-cyan-300",
    xxx: "bg-pink-500/15 text-pink-800 dark:text-pink-300",
  };
  const color = colors[ct] || "bg-surface text-secondary";
  const label = ct.replace("_", " ");
  return (
    <span className={`px-1.5 py-0.5 rounded text-xs font-medium ${color}`}>
      {label}
    </span>
  );
}

function timeAgo(isoDate: string | null): string {
  if (!isoDate) return "";
  const diff = Date.now() - new Date(isoDate).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

const CONTENT_TYPES = [
  "",
  "movie",
  "tv_show",
  "music",
  "ebook",
  "comic",
  "audiobook",
  "game",
  "software",
];

export const IndexarrBrowse = () => {
  const API = useContext(APIContext);
  const setCloseableError = useErrorStore((s) => s.setCloseableError);

  const status = useIndexarrStore((s) => s.status);
  const identity = useIndexarrStore((s) => s.identity);
  const setIdentity = useIndexarrStore((s) => s.setIdentity);
  const showSetup = useIndexarrStore((s) => s.showSetup);
  const setShowSetup = useIndexarrStore((s) => s.setShowSetup);

  const searchQuery = useIndexarrStore((s) => s.searchQuery);
  const setSearchQuery = useIndexarrStore((s) => s.setSearchQuery);
  const searchFilters = useIndexarrStore((s) => s.searchFilters);
  const setSearchFilter = useIndexarrStore((s) => s.setSearchFilter);
  const searchResults = useIndexarrStore((s) => s.searchResults);
  const searchTotal = useIndexarrStore((s) => s.searchTotal);
  const searchLoading = useIndexarrStore((s) => s.searchLoading);
  const setSearchResults = useIndexarrStore((s) => s.setSearchResults);
  const setSearchLoading = useIndexarrStore((s) => s.setSearchLoading);

  const recentTorrents = useIndexarrStore((s) => s.recentTorrents);
  const recentLoading = useIndexarrStore((s) => s.recentLoading);
  const setRecentTorrents = useIndexarrStore((s) => s.setRecentTorrents);
  const setRecentLoading = useIndexarrStore((s) => s.setRecentLoading);

  const [localSearch, setLocalSearch] = useState(searchQuery);
  const [downloadingHash, setDownloadingHash] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<"search" | "recent">("search");

  // File selection modal state for download flow
  const [pendingMagnet, setPendingMagnet] = useState<string | null>(null);
  const [listTorrentResponse, setListTorrentResponse] =
    useState<AddTorrentResponse | null>(null);
  const [listTorrentLoading, setListTorrentLoading] = useState(false);
  const [listTorrentError, setListTorrentError] =
    useState<ErrorWithLabel | null>(null);

  const recentIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Check identity on mount
  useEffect(() => {
    IndexarrAPI.getIdentityStatus()
      .then(setIdentity)
      .catch(() => {});
  }, []);

  // Load recent torrents + poll every 60s
  const loadRecent = useCallback(async () => {
    setRecentLoading(true);
    try {
      const resp = await IndexarrAPI.getRecent(50);
      setRecentTorrents(resp.results);
    } catch {
      // silent
    } finally {
      setRecentLoading(false);
    }
  }, []);

  useEffect(() => {
    loadRecent();
    recentIntervalRef.current = setInterval(loadRecent, 60_000);
    return () => {
      if (recentIntervalRef.current) clearInterval(recentIntervalRef.current);
    };
  }, [loadRecent]);

  // Search
  const doSearch = useCallback(
    async (query: string, filters: Record<string, string>) => {
      setSearchLoading(true);
      try {
        const resp = await IndexarrAPI.search(query, filters);
        setSearchResults(resp.results, resp.total, resp.offset);
      } catch (e: any) {
        setCloseableError({
          text: "Indexarr search failed",
          details: e,
        });
      } finally {
        setSearchLoading(false);
      }
    },
    [],
  );

  // eslint-disable-next-line react-hooks/exhaustive-deps
  const debouncedSearch = useCallback(
    debounce((q: string, f: Record<string, string>) => doSearch(q, f), 300),
    [doSearch],
  );

  const handleSearchChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const value = e.target.value;
    setLocalSearch(value);
    setSearchQuery(value);
    debouncedSearch(value, searchFilters);
  };

  const handleFilterChange = (key: string, value: string) => {
    setSearchFilter(key, value);
    const newFilters = { ...searchFilters, [key]: value };
    debouncedSearch(searchQuery, newFilters);
  };

  const clearSearch = () => {
    setLocalSearch("");
    setSearchQuery("");
    setSearchResults([], 0, 0);
  };

  // When pendingMagnet is set, fetch file list for the FileSelectionModal
  useEffect(() => {
    if (!pendingMagnet) return;

    let cancelled = false;
    setListTorrentLoading(true);
    setListTorrentResponse(null);
    setListTorrentError(null);

    (async () => {
      try {
        const response = await API.uploadTorrent(pendingMagnet, {
          list_only: true,
        });
        if (!cancelled) setListTorrentResponse(response);
      } catch (e) {
        if (!cancelled)
          setListTorrentError({
            text: "Error listing torrent files",
            details: e as ErrorDetails,
          });
      } finally {
        if (!cancelled) setListTorrentLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [pendingMagnet]);

  const clearPendingDownload = () => {
    setPendingMagnet(null);
    setListTorrentResponse(null);
    setListTorrentError(null);
    setListTorrentLoading(false);
    setDownloadingHash(null);
  };

  // Download handler — constructs magnet and opens file selection modal
  const handleDownload = (
    infoHash: string,
    name: string | null,
    trackers: string[] | null,
  ) => {
    setDownloadingHash(infoHash);
    const magnet = buildMagnet(infoHash, name, trackers);
    setPendingMagnet(magnet);
  };

  // Not connected
  if (status && !status.enabled) {
    return (
      <div className="h-full flex items-center justify-center text-secondary">
        <p>Indexarr integration is not enabled.</p>
      </div>
    );
  }

  if (status && status.enabled && !status.reachable) {
    return (
      <div className="h-full flex items-center justify-center text-secondary">
        <div className="text-center">
          <p className="text-lg font-semibold mb-2">Cannot reach Indexarr</p>
          <p className="text-sm">{status.error}</p>
        </div>
      </div>
    );
  }

  // Setup needed
  if (identity && identity.needs_onboarding) {
    return <IndexarrSetup />;
  }

  // Show setup panel
  if (showSetup) {
    return <IndexarrSetup />;
  }

  const cellBase = "px-3 py-2 text-sm";
  const headerCell = `${cellBase} font-medium text-secondary text-left`;
  const numCell = `${cellBase} text-right whitespace-nowrap`;

  return (
    <div className="h-full flex flex-col overflow-hidden">
      {/* Header bar */}
      <div className="flex items-center gap-2 px-4 py-2 border-b border-divider bg-surface-raised">
        {/* Tab buttons */}
        <button
          onClick={() => setActiveTab("search")}
          className={`px-3 py-1 text-sm rounded cursor-pointer ${
            activeTab === "search"
              ? "bg-primary text-white"
              : "text-secondary hover:text-text"
          }`}
        >
          Search
        </button>
        <button
          onClick={() => setActiveTab("recent")}
          className={`px-3 py-1 text-sm rounded cursor-pointer ${
            activeTab === "recent"
              ? "bg-primary text-white"
              : "text-secondary hover:text-text"
          }`}
        >
          Recent
        </button>

        <div className="flex-1" />

        {/* Setup gear */}
        <button
          onClick={() => setShowSetup(true)}
          className="p-1 text-secondary hover:text-text cursor-pointer"
          title="Indexarr Settings"
        >
          <FaCog className="w-4 h-4" />
        </button>
      </div>

      {activeTab === "search" && (
        <div className="flex-1 min-h-0 flex flex-col">
          {/* Search input + filters */}
          <div className="px-4 py-3 border-b border-divider space-y-2">
            <div className="flex gap-2">
              <div className="relative flex-1">
                <FaSearch className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-tertiary" />
                <input
                  type="text"
                  value={localSearch}
                  onChange={handleSearchChange}
                  placeholder="Search the torrent index..."
                  className="w-full pl-9 pr-8 py-2 text-sm bg-surface border border-divider rounded-lg focus:outline-none focus:border-primary placeholder:text-tertiary"
                />
                {localSearch && (
                  <button
                    onClick={clearSearch}
                    className="absolute right-2 top-1/2 -translate-y-1/2 p-0.5 text-tertiary hover:text-secondary rounded cursor-pointer"
                  >
                    <GoX className="w-4 h-4" />
                  </button>
                )}
              </div>
            </div>
            {/* Filters row */}
            <div className="flex gap-2 flex-wrap">
              <select
                value={searchFilters.content_type || ""}
                onChange={(e) =>
                  handleFilterChange("content_type", e.target.value)
                }
                className="text-xs px-2 py-1 bg-surface border border-divider rounded cursor-pointer"
              >
                <option value="">All types</option>
                {CONTENT_TYPES.filter(Boolean).map((ct) => (
                  <option key={ct} value={ct}>
                    {ct.replace("_", " ")}
                  </option>
                ))}
              </select>
              <select
                value={searchFilters.resolution || ""}
                onChange={(e) =>
                  handleFilterChange("resolution", e.target.value)
                }
                className="text-xs px-2 py-1 bg-surface border border-divider rounded cursor-pointer"
              >
                <option value="">Any resolution</option>
                {["720p", "1080p", "2160p"].map((r) => (
                  <option key={r} value={r}>
                    {r}
                  </option>
                ))}
              </select>
              <select
                value={searchFilters.sort || "relevance"}
                onChange={(e) => handleFilterChange("sort", e.target.value)}
                className="text-xs px-2 py-1 bg-surface border border-divider rounded cursor-pointer"
              >
                <option value="relevance">Sort: Relevance</option>
                <option value="date">Sort: Date</option>
                <option value="seeders">Sort: Seeders</option>
                <option value="size">Sort: Size</option>
                <option value="name">Sort: Name</option>
              </select>
            </div>
          </div>

          {/* Results */}
          <div className="flex-1 min-h-0 overflow-auto">
            {searchLoading && searchResults.length === 0 ? (
              <div className="flex items-center justify-center py-12">
                <Spinner label="Searching" />
              </div>
            ) : searchResults.length === 0 && searchQuery ? (
              <div className="flex items-center justify-center py-12 text-secondary">
                No results found
              </div>
            ) : searchResults.length === 0 ? (
              <div className="flex items-center justify-center py-12 text-secondary">
                Enter a search query to find torrents
              </div>
            ) : (
              <>
                <div className="px-4 py-1 text-xs text-secondary">
                  {searchTotal.toLocaleString()} results
                  {searchLoading && " (loading...)"}
                </div>
                <table className="w-full">
                  <thead>
                    <tr className="border-b border-divider">
                      <th className={headerCell}>Name</th>
                      <th
                        className={`${headerCell} text-right hidden md:table-cell`}
                      >
                        Size
                      </th>
                      <th
                        className={`${headerCell} text-right hidden lg:table-cell`}
                      >
                        SE
                      </th>
                      <th
                        className={`${headerCell} text-right hidden lg:table-cell`}
                      >
                        LE
                      </th>
                      <th
                        className={`${headerCell} text-right hidden md:table-cell`}
                      >
                        Age
                      </th>
                      <th className={`${headerCell} w-20`}></th>
                    </tr>
                  </thead>
                  <tbody>
                    {searchResults.map((r) => (
                      <SearchRow
                        key={r.info_hash}
                        result={r}
                        downloading={downloadingHash === r.info_hash}
                        onDownload={() =>
                          handleDownload(r.info_hash, r.name, r.trackers)
                        }
                      />
                    ))}
                  </tbody>
                </table>
              </>
            )}
          </div>
        </div>
      )}

      {activeTab === "recent" && (
        <div className="flex-1 min-h-0 overflow-auto">
          {recentLoading && recentTorrents.length === 0 ? (
            <div className="flex items-center justify-center py-12">
              <Spinner label="Loading recent" />
            </div>
          ) : recentTorrents.length === 0 ? (
            <div className="flex items-center justify-center py-12 text-secondary">
              No recent torrents
            </div>
          ) : (
            <table className="w-full">
              <thead>
                <tr className="border-b border-divider">
                  <th className={headerCell}>Name</th>
                  <th
                    className={`${headerCell} text-right hidden md:table-cell`}
                  >
                    Size
                  </th>
                  <th
                    className={`${headerCell} text-right hidden lg:table-cell`}
                  >
                    SE
                  </th>
                  <th
                    className={`${headerCell} text-right hidden md:table-cell`}
                  >
                    Age
                  </th>
                  <th className={`${headerCell} w-20`}></th>
                </tr>
              </thead>
              <tbody>
                {recentTorrents.map((r) => (
                  <RecentRow
                    key={r.info_hash}
                    item={r}
                    downloading={downloadingHash === r.info_hash}
                    onDownload={() =>
                      handleDownload(r.info_hash, r.name, r.trackers)
                    }
                  />
                ))}
              </tbody>
            </table>
          )}
        </div>
      )}

      {/* File selection modal for downloads */}
      {pendingMagnet && (
        <FileSelectionModal
          onHide={clearPendingDownload}
          listTorrentResponse={listTorrentResponse}
          listTorrentError={listTorrentError}
          listTorrentLoading={listTorrentLoading}
          data={pendingMagnet}
        />
      )}
    </div>
  );
};

// --- Row components ---

const SearchRow = ({
  result,
  downloading,
  onDownload,
}: {
  result: IndexarrSearchResult;
  downloading: boolean;
  onDownload: () => void;
}) => {
  const cellBase = "px-3 py-2 text-sm";
  const numCell = `${cellBase} text-right whitespace-nowrap`;

  return (
    <tr className="border-b border-divider hover:bg-surface-raised/50 transition-colors">
      <td className={`${cellBase} max-w-0`}>
        <div className="flex items-center gap-2 min-w-0">
          <div className="min-w-0 flex-1">
            <div className="truncate text-text" title={result.name || ""}>
              {result.name || result.info_hash}
            </div>
            <div className="flex gap-1 mt-0.5 flex-wrap">
              {contentTypeBadge(result.content_type)}
              {result.resolution && (
                <span className="px-1.5 py-0.5 rounded text-xs bg-surface text-secondary">
                  {result.resolution}
                </span>
              )}
              {result.codec && (
                <span className="px-1.5 py-0.5 rounded text-xs bg-surface text-secondary">
                  {result.codec}
                </span>
              )}
              {result.year && (
                <span className="px-1.5 py-0.5 rounded text-xs bg-surface text-secondary">
                  {result.year}
                </span>
              )}
            </div>
          </div>
        </div>
      </td>
      <td className={`${numCell} hidden md:table-cell text-secondary`}>
        {result.size ? formatBytes(result.size) : "-"}
      </td>
      <td className={`${numCell} hidden lg:table-cell`}>
        <span className="text-success inline-flex items-center gap-0.5">
          <FaSeedling className="w-3 h-3" />
          {result.seed_count}
        </span>
      </td>
      <td className={`${numCell} hidden lg:table-cell`}>
        <span className="text-secondary inline-flex items-center gap-0.5">
          <FaUsers className="w-3 h-3" />
          {result.peer_count}
        </span>
      </td>
      <td className={`${numCell} hidden md:table-cell text-secondary`}>
        {timeAgo(result.resolved_at)}
      </td>
      <td className={`${cellBase} text-right`}>
        <button
          onClick={onDownload}
          disabled={downloading}
          className="inline-flex items-center gap-1 px-2.5 py-1 text-xs font-medium rounded bg-primary text-white hover:bg-primary/80 disabled:opacity-50 cursor-pointer"
          title="Download torrent"
        >
          {downloading ? (
            <span className="w-3 h-3 border-2 border-white/30 border-t-white rounded-full animate-spin" />
          ) : (
            <FaDownload className="w-3 h-3" />
          )}
        </button>
      </td>
    </tr>
  );
};

const RecentRow = ({
  item,
  downloading,
  onDownload,
}: {
  item: IndexarrRecentItem;
  downloading: boolean;
  onDownload: () => void;
}) => {
  const cellBase = "px-3 py-2 text-sm";
  const numCell = `${cellBase} text-right whitespace-nowrap`;

  return (
    <tr className="border-b border-divider hover:bg-surface-raised/50 transition-colors">
      <td className={`${cellBase} max-w-0`}>
        <div className="min-w-0">
          <div className="truncate text-text" title={item.name || ""}>
            {item.name || item.info_hash}
          </div>
          <div className="flex gap-1 mt-0.5 flex-wrap">
            {contentTypeBadge(item.content_type)}
            {item.resolution && (
              <span className="px-1.5 py-0.5 rounded text-xs bg-surface text-secondary">
                {item.resolution}
              </span>
            )}
          </div>
        </div>
      </td>
      <td className={`${numCell} hidden md:table-cell text-secondary`}>
        {item.size ? formatBytes(item.size) : "-"}
      </td>
      <td className={`${numCell} hidden lg:table-cell`}>
        <span className="text-success inline-flex items-center gap-0.5">
          <FaSeedling className="w-3 h-3" />
          {item.seed_count}
        </span>
      </td>
      <td className={`${numCell} hidden md:table-cell text-secondary`}>
        {timeAgo(item.resolved_at)}
      </td>
      <td className={`${cellBase} text-right`}>
        <button
          onClick={onDownload}
          disabled={downloading}
          className="inline-flex items-center gap-1 px-2.5 py-1 text-xs font-medium rounded bg-primary text-white hover:bg-primary/80 disabled:opacity-50 cursor-pointer"
          title="Download torrent"
        >
          {downloading ? (
            <span className="w-3 h-3 border-2 border-white/30 border-t-white rounded-full animate-spin" />
          ) : (
            <FaDownload className="w-3 h-3" />
          )}
        </button>
      </td>
    </tr>
  );
};
