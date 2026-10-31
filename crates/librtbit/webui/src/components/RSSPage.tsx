import { useCallback, useEffect, useMemo, useState } from "react";
import {
  BsPlus,
  BsPencil,
  BsTrash,
  BsDownload,
  BsCheckCircleFill,
  BsRss,
  BsFilter,
} from "react-icons/bs";
import { RssAPI } from "../http-api";
import { formatDuration } from "../helper/formatDuration.js";
import { useRssStore } from "../stores/rssStore";
import { useErrorStore } from "../stores/errorStore";
import {
  RssFeedConfig,
  RssItem,
  RssRule,
  RssRuleEpisode,
  ErrorDetails,
} from "../api-types";
import {
  episodeMarker,
  matchedDate,
  ruleTracksEpisodes,
} from "../helper/rssEpisodes";
import { Spinner } from "./Spinner";
import { formatBytes } from "../helper/formatBytes";

// ---------------------------------------------------------------------------
// Shared styles
// ---------------------------------------------------------------------------

const fieldCls =
  "bg-surface border border-divider rounded focus-ring focus:border-primary";
const inputCls = `w-full px-2 py-1.5 text-sm ${fieldCls}`;
const selectSmCls = `text-xs px-2 py-1 ${fieldCls}`;
const labelCls = "block text-xs font-medium text-secondary mb-1";
const errorCls = "text-xs text-error bg-error-bg/10 px-2 py-1 rounded";

const enabledBadgeCls = (enabled: boolean) =>
  `text-xs px-1.5 py-0.5 rounded ${
    enabled
      ? "bg-success-bg/15 text-success"
      : "bg-surface-sunken text-secondary"
  }`;

// ---------------------------------------------------------------------------
// Narrow layout: below `sm` the tables become stacked rows
// (title, then a meta line, then status and actions).
// ---------------------------------------------------------------------------

const StackedList: React.FC<{
  label: string;
  empty: string;
  children: React.ReactNode[];
}> = ({ label, empty, children }) => (
  <ul aria-label={label} className="sm:hidden divide-y divide-divider">
    {children.length > 0 ? (
      children
    ) : (
      <li className="px-4 py-8 text-center text-secondary text-sm">{empty}</li>
    )}
  </ul>
);

const StackedRow: React.FC<{
  title: string;
  meta: (string | null | undefined)[];
  status: React.ReactNode;
  actions: React.ReactNode;
}> = ({ title, meta, status, actions }) => (
  <li className="px-3 py-2 space-y-1">
    <div className="text-sm font-medium break-words">{title}</div>
    <div className="text-xs text-secondary truncate">
      {meta.filter(Boolean).join(" · ")}
    </div>
    <div className="flex items-center justify-between gap-2">
      {status}
      <div className="flex gap-1">{actions}</div>
    </div>
  </li>
);

// ---------------------------------------------------------------------------
// Feed Modal
// ---------------------------------------------------------------------------

const FeedModal: React.FC<{
  isOpen: boolean;
  onClose: () => void;
  feed?: RssFeedConfig | null;
  onSaved: () => void;
}> = ({ isOpen, onClose, feed, onSaved }) => {
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [pollInterval, setPollInterval] = useState(900);
  const [category, setCategory] = useState("");
  const [filterRegex, setFilterRegex] = useState("");
  const [enabled, setEnabled] = useState(true);
  const [autoDownload, setAutoDownload] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (feed) {
      setName(feed.name);
      setUrl(feed.url);
      setPollInterval(feed.poll_interval_secs);
      setCategory(feed.category ?? "");
      setFilterRegex(feed.filter_regex ?? "");
      setEnabled(feed.enabled);
      setAutoDownload(feed.auto_download);
    } else {
      setName("");
      setUrl("");
      setPollInterval(900);
      setCategory("");
      setFilterRegex("");
      setEnabled(true);
      setAutoDownload(false);
    }
    setError(null);
  }, [feed, isOpen]);

  if (!isOpen) return null;

  const handleSave = async () => {
    if (!name.trim() || !url.trim()) {
      setError("Name and URL are required");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const feedConfig: RssFeedConfig = {
        name: name.trim(),
        url: url.trim(),
        poll_interval_secs: pollInterval,
        category: category.trim() || null,
        filter_regex: filterRegex.trim() || null,
        enabled,
        auto_download: autoDownload,
      };
      if (feed) {
        await RssAPI.updateFeed(feed.name, feedConfig);
      } else {
        await RssAPI.addFeed(feedConfig);
      }
      onSaved();
      onClose();
    } catch (e: any) {
      setError(e?.text || e?.message || "Failed to save feed");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="bg-surface-raised rounded-lg shadow-xl w-full max-w-md mx-4">
        <div className="px-4 py-3 border-b border-divider">
          <h2 className="text-base font-semibold">
            {feed ? "Edit Feed" : "Add Feed"}
          </h2>
        </div>
        <div className="px-4 py-3 space-y-3">
          {error && (
            <div className={errorCls} role="alert">
              {error}
            </div>
          )}
          <div>
            <label className={labelCls}>Name</label>
            <input
              className={inputCls}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="My Feed"
              disabled={!!feed}
            />
          </div>
          <div>
            <label className={labelCls}>URL</label>
            <input
              className={inputCls}
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="https://example.com/rss"
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className={labelCls}>Poll Interval (sec)</label>
              <input
                type="number"
                className={inputCls}
                value={pollInterval}
                onChange={(e) => setPollInterval(Number(e.target.value) || 900)}
              />
            </div>
            <div>
              <label className={labelCls}>Category</label>
              <input
                className={inputCls}
                value={category}
                onChange={(e) => setCategory(e.target.value)}
                placeholder="Optional"
              />
            </div>
          </div>
          <div>
            <label className={labelCls}>Filter Regex</label>
            <input
              className={inputCls}
              value={filterRegex}
              onChange={(e) => setFilterRegex(e.target.value)}
              placeholder="Optional title filter"
            />
          </div>
          <div className="flex items-center gap-4">
            <label className="flex items-center gap-1.5 text-sm">
              <input
                type="checkbox"
                checked={enabled}
                onChange={(e) => setEnabled(e.target.checked)}
              />
              Enabled
            </label>
            <label className="flex items-center gap-1.5 text-sm">
              <input
                type="checkbox"
                checked={autoDownload}
                onChange={(e) => setAutoDownload(e.target.checked)}
              />
              Auto-download
            </label>
          </div>
        </div>
        <div className="flex justify-end gap-2 px-4 py-3 border-t border-divider">
          <button
            onClick={onClose}
            className="px-3 py-1.5 text-sm text-secondary hover:text-text rounded border border-divider cursor-pointer"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={saving}
            className="px-3 py-1.5 text-sm text-white bg-primary hover:bg-primary/90 rounded disabled:opacity-50 cursor-pointer"
          >
            {saving ? "Saving..." : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Rule Modal
// ---------------------------------------------------------------------------

const RuleModal: React.FC<{
  isOpen: boolean;
  onClose: () => void;
  rule?: RssRule | null;
  feedNames: string[];
  onSaved: () => void;
}> = ({ isOpen, onClose, rule, feedNames, onSaved }) => {
  const [name, setName] = useState("");
  const [selectedFeeds, setSelectedFeeds] = useState<string[]>([]);
  const [category, setCategory] = useState("");
  const [priority, setPriority] = useState(1);
  const [matchRegex, setMatchRegex] = useState("");
  const [enabled, setEnabled] = useState(true);
  const [trackEpisodes, setTrackEpisodes] = useState(true);
  const [episodes, setEpisodes] = useState<RssRuleEpisode[] | null>(null);
  const [episodesError, setEpisodesError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setEpisodes(null);
    setEpisodesError(null);
    if (!rule || !isOpen) return;
    let cancelled = false;
    RssAPI.getRuleEpisodes(rule.id)
      .then((list) => {
        if (!cancelled) setEpisodes(list);
      })
      .catch((e: any) => {
        if (!cancelled)
          setEpisodesError(e?.text || e?.message || "Failed to load episodes");
      });
    return () => {
      cancelled = true;
    };
  }, [rule, isOpen]);

  useEffect(() => {
    if (rule) {
      setName(rule.name);
      setSelectedFeeds(rule.feed_names);
      setCategory(rule.category ?? "");
      setPriority(rule.priority);
      setMatchRegex(rule.match_regex);
      setEnabled(rule.enabled);
      setTrackEpisodes(ruleTracksEpisodes(rule));
    } else {
      setName("");
      setSelectedFeeds([]);
      setCategory("");
      setPriority(1);
      setMatchRegex("");
      setEnabled(true);
      setTrackEpisodes(true);
    }
    setError(null);
  }, [rule, isOpen]);

  if (!isOpen) return null;

  const toggleFeed = (feed: string) => {
    setSelectedFeeds((prev) =>
      prev.includes(feed) ? prev.filter((f) => f !== feed) : [...prev, feed],
    );
  };

  const handleSave = async () => {
    if (!name.trim() || !matchRegex.trim()) {
      setError("Name and match regex are required");
      return;
    }
    if (selectedFeeds.length === 0) {
      setError("Select at least one feed");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const body = {
        name: name.trim(),
        feed_names: selectedFeeds,
        category: category.trim() || null,
        priority,
        match_regex: matchRegex.trim(),
        enabled,
        track_episodes: trackEpisodes,
      };
      if (rule) {
        await RssAPI.updateRule(rule.id, body);
      } else {
        await RssAPI.addRule(body);
      }
      onSaved();
      onClose();
    } catch (e: any) {
      setError(e?.text || e?.message || "Failed to save rule");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="bg-surface-raised rounded-lg shadow-xl w-full max-w-md mx-4">
        <div className="px-4 py-3 border-b border-divider">
          <h2 className="text-base font-semibold">
            {rule ? "Edit Rule" : "Add Rule"}
          </h2>
        </div>
        <div className="px-4 py-3 space-y-3">
          {error && (
            <div className={errorCls} role="alert">
              {error}
            </div>
          )}
          <div>
            <label className={labelCls}>Name</label>
            <input
              className={inputCls}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Rule name"
            />
          </div>
          <div>
            <label className={labelCls}>Match Regex</label>
            <input
              className={inputCls}
              value={matchRegex}
              onChange={(e) => setMatchRegex(e.target.value)}
              placeholder="e.g. 1080p.*HEVC"
            />
          </div>
          <div>
            <label className={labelCls}>Feeds</label>
            <div className="flex flex-wrap gap-1.5">
              {feedNames.map((f) => (
                <label
                  key={f}
                  className="flex items-center gap-1 text-xs bg-surface px-2 py-1 rounded border border-divider"
                >
                  <input
                    type="checkbox"
                    checked={selectedFeeds.includes(f)}
                    onChange={() => toggleFeed(f)}
                  />
                  {f}
                </label>
              ))}
              {feedNames.length === 0 && (
                <span className="text-xs text-tertiary">
                  No feeds configured
                </span>
              )}
            </div>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className={labelCls}>Category</label>
              <input
                className={inputCls}
                value={category}
                onChange={(e) => setCategory(e.target.value)}
                placeholder="Optional"
              />
            </div>
            <div>
              <label className={labelCls}>Priority</label>
              <select
                className={inputCls}
                value={priority}
                onChange={(e) => setPriority(Number(e.target.value))}
              >
                <option value={0}>Low</option>
                <option value={1}>Normal</option>
                <option value={2}>High</option>
                <option value={3}>Force</option>
              </select>
            </div>
          </div>
          <label className="flex items-center gap-1.5 text-sm">
            <input
              type="checkbox"
              checked={enabled}
              onChange={(e) => setEnabled(e.target.checked)}
            />
            Enabled
          </label>
          <label className="flex items-center gap-1.5 text-sm">
            <input
              type="checkbox"
              checked={trackEpisodes}
              onChange={(e) => setTrackEpisodes(e.target.checked)}
            />
            Skip episodes already downloaded
          </label>
          {rule && (
            <div>
              <label className={labelCls}>Matched episodes</label>
              {episodesError ? (
                <div className={errorCls} role="alert">
                  {episodesError}
                </div>
              ) : episodes === null ? (
                <span className="text-xs text-tertiary">Loading…</span>
              ) : episodes.length === 0 ? (
                <span className="text-xs text-tertiary">
                  No episodes matched yet
                </span>
              ) : (
                <ul className="max-h-40 overflow-y-auto text-xs divide-y divide-divider border border-divider rounded">
                  {episodes.map((ep, i) => (
                    <li
                      key={`${ep.matched_at}-${i}`}
                      className="flex gap-2 px-2 py-1"
                    >
                      <span className="font-mono shrink-0">
                        {episodeMarker(ep)}
                      </span>
                      <span className="truncate flex-1" title={ep.title}>
                        {ep.title}
                      </span>
                      <span className="text-tertiary shrink-0">
                        {matchedDate(ep.matched_at)}
                      </span>
                      {ep.torrent_id !== null && (
                        <span className="text-tertiary shrink-0">
                          #{ep.torrent_id}
                        </span>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
        </div>
        <div className="flex justify-end gap-2 px-4 py-3 border-t border-divider">
          <button
            onClick={onClose}
            className="px-3 py-1.5 text-sm text-secondary hover:text-text rounded border border-divider cursor-pointer"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={saving}
            className="px-3 py-1.5 text-sm text-white bg-primary hover:bg-primary/90 rounded disabled:opacity-50 cursor-pointer"
          >
            {saving ? "Saving..." : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Main RSS Page
// ---------------------------------------------------------------------------

type RssTab = "items" | "feeds" | "rules";

export const RSSPage: React.FC = () => {
  const feeds = useRssStore((s) => s.feeds);
  const setFeeds = useRssStore((s) => s.setFeeds);
  const items = useRssStore((s) => s.items);
  const setItems = useRssStore((s) => s.setItems);
  const rules = useRssStore((s) => s.rules);
  const setRules = useRssStore((s) => s.setRules);
  const selectedFeedFilter = useRssStore((s) => s.selectedFeedFilter);
  const setSelectedFeedFilter = useRssStore((s) => s.setSelectedFeedFilter);
  const loading = useRssStore((s) => s.loading);
  const setLoading = useRssStore((s) => s.setLoading);
  const setCloseableError = useErrorStore((s) => s.setCloseableError);

  const [tab, setTab] = useState<RssTab>("items");
  const [feedModalOpen, setFeedModalOpen] = useState(false);
  const [editingFeed, setEditingFeed] = useState<RssFeedConfig | null>(null);
  const [ruleModalOpen, setRuleModalOpen] = useState(false);
  const [editingRule, setEditingRule] = useState<RssRule | null>(null);
  const [downloadingId, setDownloadingId] = useState<string | null>(null);

  const loadAll = useCallback(async () => {
    setLoading(true);
    try {
      const [feedsRes, itemsRes, rulesRes] = await Promise.all([
        RssAPI.getFeeds(),
        RssAPI.getItems(selectedFeedFilter ?? undefined),
        RssAPI.getRules(),
      ]);
      setFeeds(feedsRes);
      setItems(itemsRes);
      setRules(rulesRes);
    } catch (e: any) {
      setCloseableError({
        text: "Error loading RSS data",
        details: e as ErrorDetails,
      });
    } finally {
      setLoading(false);
    }
  }, [selectedFeedFilter]);

  useEffect(() => {
    loadAll();
    const interval = setInterval(loadAll, 30000);
    return () => clearInterval(interval);
  }, [loadAll]);

  const handleDeleteFeed = async (name: string) => {
    try {
      await RssAPI.deleteFeed(name);
      loadAll();
    } catch (e: any) {
      setCloseableError({ text: "Error deleting feed", details: e });
    }
  };

  const handleDeleteRule = async (id: string) => {
    try {
      await RssAPI.deleteRule(id);
      loadAll();
    } catch (e: any) {
      setCloseableError({ text: "Error deleting rule", details: e });
    }
  };

  const handleDownload = async (item: RssItem) => {
    setDownloadingId(item.id);
    try {
      await RssAPI.downloadItem(item.id);
      loadAll();
    } catch (e: any) {
      setCloseableError({ text: "Error downloading item", details: e });
    } finally {
      setDownloadingId(null);
    }
  };

  const feedNames = useMemo(() => feeds.map((f) => f.name), [feeds]);

  const formatDate = (s?: string | null) => {
    if (!s) return "-";
    const d = new Date(s);
    return d.toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  };

  const tabCls = (t: RssTab) =>
    `px-3 py-1.5 text-sm font-medium border-b-2 cursor-pointer transition-colors focus-ring ${
      tab === t
        ? "border-primary text-primary"
        : "border-transparent text-secondary hover:text-text hover:border-divider"
    }`;

  const btnSmCls =
    "p-1 text-secondary hover:text-primary rounded cursor-pointer transition-colors focus-ring touch-target";
  const thCls =
    "px-2 py-1.5 text-left text-xs font-medium text-secondary uppercase tracking-wider";
  const tdCls = "px-2 py-1.5 text-sm whitespace-nowrap";

  const emptyCls = "px-4 py-8 text-center text-secondary text-sm";
  const itemsEmpty = "No RSS items found. Add a feed to get started.";
  const feedsEmpty = 'No RSS feeds configured. Click "Add Feed" to add one.';
  const rulesEmpty = 'No download rules. Click "Add Rule" to create one.';

  const itemSize = (item: RssItem) =>
    item.size_bytes > 0 ? formatBytes(item.size_bytes) : "-";

  const itemStatus = (item: RssItem) =>
    item.downloaded ? (
      <span className="flex items-center gap-1 text-success text-xs">
        <BsCheckCircleFill className="w-3 h-3" /> Done
      </span>
    ) : (
      <span className="text-xs text-secondary">Pending</span>
    );

  const itemActions = (item: RssItem) =>
    !item.downloaded &&
    item.url && (
      <button
        onClick={() => handleDownload(item)}
        disabled={downloadingId === item.id}
        className={btnSmCls}
        title="Download"
        aria-label={`Download ${item.title}`}
      >
        {downloadingId === item.id ? (
          <Spinner />
        ) : (
          <BsDownload className="w-3.5 h-3.5" />
        )}
      </button>
    );

  const enabledBadge = (enabled: boolean) => (
    <span className={enabledBadgeCls(enabled)}>
      {enabled ? "Active" : "Disabled"}
    </span>
  );

  const feedActions = (feed: RssFeedConfig) => (
    <>
      <button
        onClick={() => {
          setEditingFeed(feed);
          setFeedModalOpen(true);
        }}
        className={btnSmCls}
        title="Edit"
        aria-label={`Edit feed ${feed.name}`}
      >
        <BsPencil className="w-3.5 h-3.5" />
      </button>
      <button
        onClick={() => handleDeleteFeed(feed.name)}
        className={`${btnSmCls} hover:text-error`}
        title="Delete"
        aria-label={`Delete feed ${feed.name}`}
      >
        <BsTrash className="w-3.5 h-3.5" />
      </button>
    </>
  );

  const ruleActions = (rule: RssRule) => (
    <>
      <button
        onClick={() => {
          setEditingRule(rule);
          setRuleModalOpen(true);
        }}
        className={btnSmCls}
        title="Edit"
        aria-label={`Edit rule ${rule.name}`}
      >
        <BsPencil className="w-3.5 h-3.5" />
      </button>
      <button
        onClick={() => handleDeleteRule(rule.id)}
        className={`${btnSmCls} hover:text-error`}
        title="Delete"
        aria-label={`Delete rule ${rule.name}`}
      >
        <BsTrash className="w-3.5 h-3.5" />
      </button>
    </>
  );

  if (loading && items.length === 0) {
    return (
      <div className="flex justify-center items-center h-full">
        <Spinner />
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col">
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-divider">
        <div className="flex items-center gap-1">
          <button onClick={() => setTab("items")} className={tabCls("items")}>
            Items
          </button>
          <button onClick={() => setTab("feeds")} className={tabCls("feeds")}>
            Feeds ({feeds.length})
          </button>
          <button onClick={() => setTab("rules")} className={tabCls("rules")}>
            Rules ({rules.length})
          </button>
        </div>
        <div className="flex items-center gap-2">
          {tab === "items" && (
            <select
              className={selectSmCls}
              aria-label="Filter by feed"
              value={selectedFeedFilter ?? ""}
              onChange={(e) => setSelectedFeedFilter(e.target.value || null)}
            >
              <option value="">All feeds</option>
              {feedNames.map((f) => (
                <option key={f} value={f}>
                  {f}
                </option>
              ))}
            </select>
          )}
          {tab === "feeds" && (
            <button
              onClick={() => {
                setEditingFeed(null);
                setFeedModalOpen(true);
              }}
              className="flex items-center gap-1 px-2 py-1 text-xs text-white bg-primary-bg rounded hover:bg-primary-bg-hover cursor-pointer focus-ring"
            >
              <BsPlus className="w-4 h-4" /> Add Feed
            </button>
          )}
          {tab === "rules" && (
            <button
              onClick={() => {
                setEditingRule(null);
                setRuleModalOpen(true);
              }}
              className="flex items-center gap-1 px-2 py-1 text-xs text-white bg-primary-bg rounded hover:bg-primary-bg-hover cursor-pointer focus-ring"
            >
              <BsPlus className="w-4 h-4" /> Add Rule
            </button>
          )}
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 min-h-0 overflow-auto">
        {tab === "items" && (
          <>
            <table className="w-full hidden sm:table">
              <thead className="sticky top-0 bg-surface-raised z-10">
                <tr className="border-b border-divider">
                  <th className={thCls}>Title</th>
                  <th className={`${thCls} hidden lg:table-cell`}>Feed</th>
                  <th className={`${thCls} hidden lg:table-cell`}>Published</th>
                  <th className={`${thCls} hidden md:table-cell`}>Size</th>
                  <th className={thCls}>Status</th>
                  <th className={thCls} style={{ width: 40 }}></th>
                </tr>
              </thead>
              <tbody>
                {items.map((item) => (
                  <tr
                    key={item.id}
                    className="border-b border-divider hover:bg-surface/50"
                  >
                    <td
                      className={`${tdCls} max-w-xs truncate`}
                      title={item.title}
                    >
                      {item.title}
                    </td>
                    <td
                      className={`${tdCls} hidden lg:table-cell text-secondary`}
                    >
                      {item.feed_name}
                    </td>
                    <td
                      className={`${tdCls} hidden lg:table-cell text-secondary`}
                    >
                      {formatDate(item.published_at)}
                    </td>
                    <td
                      className={`${tdCls} hidden md:table-cell text-secondary`}
                    >
                      {itemSize(item)}
                    </td>
                    <td className={tdCls}>{itemStatus(item)}</td>
                    <td className={tdCls}>{itemActions(item)}</td>
                  </tr>
                ))}
                {items.length === 0 && (
                  <tr>
                    <td colSpan={6} className={emptyCls}>
                      {itemsEmpty}
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
            <StackedList label="RSS items" empty={itemsEmpty}>
              {items.map((item) => (
                <StackedRow
                  key={item.id}
                  title={item.title}
                  meta={[
                    item.feed_name,
                    formatDate(item.published_at),
                    itemSize(item),
                  ]}
                  status={itemStatus(item)}
                  actions={itemActions(item)}
                />
              ))}
            </StackedList>
          </>
        )}

        {tab === "feeds" && (
          <>
            <table className="w-full hidden sm:table">
              <thead className="sticky top-0 bg-surface-raised z-10">
                <tr className="border-b border-divider">
                  <th className={thCls}>Name</th>
                  <th className={`${thCls} hidden lg:table-cell`}>URL</th>
                  <th className={`${thCls} hidden md:table-cell`}>Interval</th>
                  <th className={`${thCls} hidden md:table-cell`}>Category</th>
                  <th className={thCls}>Status</th>
                  <th className={thCls} style={{ width: 80 }}></th>
                </tr>
              </thead>
              <tbody>
                {feeds.map((feed) => (
                  <tr
                    key={feed.name}
                    className="border-b border-divider hover:bg-surface/50"
                  >
                    <td className={`${tdCls} font-medium`}>{feed.name}</td>
                    <td
                      className={`${tdCls} hidden lg:table-cell text-secondary max-w-xs truncate`}
                      title={feed.url}
                    >
                      {feed.url}
                    </td>
                    <td
                      className={`${tdCls} hidden md:table-cell text-secondary`}
                    >
                      {formatDuration(feed.poll_interval_secs)}
                    </td>
                    <td
                      className={`${tdCls} hidden md:table-cell text-secondary`}
                    >
                      {feed.category || "-"}
                    </td>
                    <td className={tdCls}>{enabledBadge(feed.enabled)}</td>
                    <td className={`${tdCls} flex gap-1`}>
                      {feedActions(feed)}
                    </td>
                  </tr>
                ))}
                {feeds.length === 0 && (
                  <tr>
                    <td colSpan={6} className={emptyCls}>
                      {feedsEmpty}
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
            <StackedList label="RSS feeds" empty={feedsEmpty}>
              {feeds.map((feed) => (
                <StackedRow
                  key={feed.name}
                  title={feed.name}
                  meta={[
                    feed.url,
                    formatDuration(feed.poll_interval_secs),
                    feed.category,
                  ]}
                  status={enabledBadge(feed.enabled)}
                  actions={feedActions(feed)}
                />
              ))}
            </StackedList>
          </>
        )}

        {tab === "rules" && (
          <>
            <table className="w-full hidden sm:table">
              <thead className="sticky top-0 bg-surface-raised z-10">
                <tr className="border-b border-divider">
                  <th className={thCls}>Name</th>
                  <th className={`${thCls} hidden lg:table-cell`}>Pattern</th>
                  <th className={`${thCls} hidden md:table-cell`}>Feeds</th>
                  <th className={`${thCls} hidden md:table-cell`}>Category</th>
                  <th className={thCls}>Status</th>
                  <th className={thCls} style={{ width: 80 }}></th>
                </tr>
              </thead>
              <tbody>
                {rules.map((rule) => (
                  <tr
                    key={rule.id}
                    className="border-b border-divider hover:bg-surface/50"
                  >
                    <td className={`${tdCls} font-medium`}>{rule.name}</td>
                    <td
                      className={`${tdCls} hidden lg:table-cell text-secondary font-mono text-xs max-w-xs truncate`}
                      title={rule.match_regex}
                    >
                      {rule.match_regex}
                    </td>
                    <td
                      className={`${tdCls} hidden md:table-cell text-secondary`}
                    >
                      {rule.feed_names.join(", ")}
                    </td>
                    <td
                      className={`${tdCls} hidden md:table-cell text-secondary`}
                    >
                      {rule.category || "-"}
                    </td>
                    <td className={tdCls}>{enabledBadge(rule.enabled)}</td>
                    <td className={`${tdCls} flex gap-1`}>
                      {ruleActions(rule)}
                    </td>
                  </tr>
                ))}
                {rules.length === 0 && (
                  <tr>
                    <td colSpan={6} className={emptyCls}>
                      {rulesEmpty}
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
            <StackedList label="RSS rules" empty={rulesEmpty}>
              {rules.map((rule) => (
                <StackedRow
                  key={rule.id}
                  title={rule.name}
                  meta={[
                    rule.match_regex,
                    rule.feed_names.join(", "),
                    rule.category,
                  ]}
                  status={enabledBadge(rule.enabled)}
                  actions={ruleActions(rule)}
                />
              ))}
            </StackedList>
          </>
        )}
      </div>

      {/* Modals */}
      <FeedModal
        isOpen={feedModalOpen}
        onClose={() => setFeedModalOpen(false)}
        feed={editingFeed}
        onSaved={loadAll}
      />
      <RuleModal
        isOpen={ruleModalOpen}
        onClose={() => setRuleModalOpen(false)}
        rule={editingRule}
        feedNames={feedNames}
        onSaved={loadAll}
      />
    </div>
  );
};
