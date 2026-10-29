// The torrent view encoded in the URL query so a filtered, sorted, searched
// view can be shared and restored on load. Only the torrent list view is
// encoded; other pages keep the plain path. Unknown keys are dropped.

export type ViewStatus =
  | "all"
  | "downloading"
  | "seeding"
  | "completed"
  | "running"
  | "stopped"
  | "active"
  | "stalled_up"
  | "stalled_down"
  | "checking"
  | "paused"
  | "queued"
  | "error";

export type ViewSortColumn =
  | "id"
  | "name"
  | "size"
  | "progress"
  | "downloadedBytes"
  | "downSpeed"
  | "upSpeed"
  | "downLimit"
  | "upLimit"
  | "uploadedBytes"
  | "eta"
  | "peers"
  | "state"
  | "ratio"
  | "category"
  | "seeding_time"
  | "queue_position"
  | "availability"
  | "added_on"
  | "tracker"
  | "amount_left"
  | "save_path";

export type ViewSortDirection = "asc" | "desc";

export const VIEW_STATUSES = [
  "all",
  "downloading",
  "seeding",
  "completed",
  "running",
  "stopped",
  "active",
  "stalled_up",
  "stalled_down",
  "checking",
  "paused",
  "queued",
  "error",
] as const satisfies readonly ViewStatus[];

export const TORRENT_SORT_COLUMNS = [
  "id",
  "name",
  "size",
  "progress",
  "downloadedBytes",
  "downSpeed",
  "upSpeed",
  "downLimit",
  "upLimit",
  "uploadedBytes",
  "eta",
  "peers",
  "state",
  "ratio",
  "category",
  "seeding_time",
  "queue_position",
  "availability",
  "added_on",
  "tracker",
  "amount_left",
  "save_path",
] as const satisfies readonly ViewSortColumn[];

export interface ViewQuery {
  search: string;
  status: ViewStatus;
  category: string | null;
  tracker: string | null;
  // null = all, "" = untagged, otherwise the tag name.
  tag: string | null;
  sortColumn: ViewSortColumn;
  sortDirection: ViewSortDirection;
}

export const DEFAULT_VIEW_QUERY: ViewQuery = {
  search: "",
  status: "all",
  category: null,
  tracker: null,
  tag: null,
  sortColumn: "id",
  sortDirection: "desc",
};

const STATUS_VALUES = VIEW_STATUSES as readonly string[];
const SORT_COLUMNS = TORRENT_SORT_COLUMNS as readonly string[];

function readString(params: URLSearchParams, key: string): string {
  const value = params.get(key);
  return value === null ? "" : value;
}

/** Parse a query string into a view. Invalid values fall back to defaults. */
export function parseViewQuery(search: string): ViewQuery {
  const params = new URLSearchParams(
    search.startsWith("?") ? search.slice(1) : search,
  );
  const status = readString(params, "status");
  const sort = readString(params, "sort");
  const dir = readString(params, "dir");
  const category = params.get("category");
  const tracker = params.get("tracker");
  const tag = params.get("tag");
  return {
    search: readString(params, "q"),
    status: STATUS_VALUES.includes(status)
      ? (status as ViewStatus)
      : DEFAULT_VIEW_QUERY.status,
    category: category === null || category === "" ? null : category,
    tracker: tracker === null || tracker === "" ? null : tracker,
    tag,
    sortColumn: SORT_COLUMNS.includes(sort)
      ? (sort as ViewSortColumn)
      : DEFAULT_VIEW_QUERY.sortColumn,
    sortDirection: dir === "asc" || dir === "desc" ? dir : DEFAULT_VIEW_QUERY.sortDirection,
  };
}

/** Encode a view as a query string without the leading "?". Defaults are omitted. */
export function formatViewQuery(view: ViewQuery): string {
  const params = new URLSearchParams();
  if (view.search !== "") params.set("q", view.search);
  if (view.status !== DEFAULT_VIEW_QUERY.status) params.set("status", view.status);
  if (view.category !== null) params.set("category", view.category);
  if (view.tracker !== null) params.set("tracker", view.tracker);
  if (view.tag !== null) params.set("tag", view.tag);
  if (view.sortColumn !== DEFAULT_VIEW_QUERY.sortColumn) {
    params.set("sort", view.sortColumn);
  }
  if (view.sortDirection !== DEFAULT_VIEW_QUERY.sortDirection) {
    params.set("dir", view.sortDirection);
  }
  return params.toString();
}

export function viewsEqual(a: ViewQuery, b: ViewQuery): boolean {
  return (
    a.search === b.search &&
    a.status === b.status &&
    a.category === b.category &&
    a.tracker === b.tracker &&
    a.tag === b.tag &&
    a.sortColumn === b.sortColumn &&
    a.sortDirection === b.sortDirection
  );
}

type HistoryLike = {
  pushState: (data: unknown, unused: string, url?: string | URL | null) => void;
  replaceState: (data: unknown, unused: string, url?: string | URL | null) => void;
};

type LocationLike = { pathname: string; search: string };

export type ViewHistoryMode = "push" | "replace";

/**
 * Write the view into the current URL. `push` (the default) adds a history
 * entry so the back and forward buttons move between views. `replace` updates
 * the current entry, used when the view is being restored from the URL.
 */
export function writeViewQuery(
  view: ViewQuery,
  location: LocationLike = window.location,
  history: HistoryLike = window.history,
  mode: ViewHistoryMode = "push",
): void {
  const query = formatViewQuery(view);
  const next = query === "" ? location.pathname : `${location.pathname}?${query}`;
  const current = `${location.pathname}${location.search}`;
  if (next === current) return;
  const write = mode === "replace" ? history.replaceState : history.pushState;
  write.call(history, null, "", next);
}
