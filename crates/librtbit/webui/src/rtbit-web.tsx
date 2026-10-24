import {
  JSX,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";
import { BsX } from "react-icons/bs";
import { ErrorDetails as ApiErrorDetails } from "./api-types";
import { APIContext } from "./context";
import { RootContent } from "./components/RootContent";
import { customSetInterval } from "./helper/customSetInterval";
import { LogStreamModal } from "./components/modal/LogStreamModal";
import { Toolbar } from "./components/Toolbar";
import { Sidebar } from "./components/Sidebar";
import { EngineUnreachable } from "./components/ErrorComponent";
import { useTorrentStore } from "./stores/torrentStore";
import { useErrorStore } from "./stores/errorStore";
import { AlertModal } from "./components/modal/AlertModal";
import { useStatsStore } from "./stores/statsStore";
import { Footer } from "./components/Footer";
import { FileSelectionModal } from "./components/modal/FileSelectionModal";
import { MultiTorrentUploadModal } from "./components/modal/MultiTorrentUploadModal";
import { useIsLargeScreen } from "./hooks/useIsLargeScreen";
import { useListOnlyUpload } from "./hooks/useListOnlyUpload";
import { useIndexarrStore } from "./stores/indexarrStore";
import { CurrentPage, useUIStore } from "./stores/uiStore";
import { useToastStore } from "./stores/toastStore";
import { ToastViewport } from "./components/Toast";
import { Button } from "./components/buttons/Button";
import { extractTorrentLink } from "./helper/magnetLink";
import { retryDelayMs } from "./helper/retryBackoff";

export interface ErrorWithLabel {
  text: string;
  details?: ApiErrorDetails;
}

export interface ContextType {
  setCloseableError: (error: ErrorWithLabel | null) => void;
  refreshTorrents: () => void;
}

type PendingUpload =
  | { type: "single"; file: File }
  | { type: "multi"; files: File[] }
  | { type: "link"; link: string }
  | null;

const isEngineUnreachable = (e: ApiErrorDetails): boolean =>
  e.status === undefined
    ? e.text === "network error"
    : e.status === 404 || e.status >= 500;

// Shown over the last known torrent list while the engine cannot be reached
const ConnectionLostBanner = (props: { onRetry: () => void }) => (
  <div
    role="status"
    aria-live="polite"
    className="absolute top-2 inset-x-2 z-40 flex items-center gap-3 rounded-md border border-error bg-surface shadow-lg px-3 py-2 text-sm"
  >
    <p className="flex-1 min-w-0 text-error">
      Connection to rustTorrent lost. Showing the last known data; retrying…
    </p>
    <Button variant="primary" onClick={props.onRetry}>
      Retry
    </Button>
  </div>
);

export const RtbitWebUI = (props: {
  title: string;
  version: string;
  menuButtons?: JSX.Element[];
}) => {
  const [logsOpened, setLogsOpened] = useState<boolean>(false);
  const setOtherError = useErrorStore((state) => state.setOtherError);
  const pushToast = useToastStore((state) => state.pushToast);

  const API = useContext(APIContext);

  const isLargeScreen = useIsLargeScreen();
  const sidebarOpen = useUIStore((state) => state.sidebarOpen);
  const setSidebarOpen = useUIStore((state) => state.setSidebarOpen);
  const currentPage = useUIStore((state) => state.currentPage);
  const setCurrentPage = useUIStore((state) => state.setCurrentPage);
  const indexarrEnabled = useIndexarrStore(
    (state) => state.status?.enabled ?? false,
  );

  const setTorrents = useTorrentStore((state) => state.setTorrents);
  const setTorrentsLoading = useTorrentStore(
    (state) => state.setTorrentsLoading,
  );
  const setRefreshTorrents = useTorrentStore(
    (state) => state.setRefreshTorrents,
  );
  // A boolean selector, so the shell does not re-render on every list update
  const hasTorrentData = useTorrentStore((state) => state.torrents !== null);

  // The engine is unreachable when the request got no response, or the server
  // answered 404 or 5xx. Other failures keep the normal error alert.
  const [engineUnreachable, setEngineUnreachable] =
    useState<ApiErrorDetails | null>(null);
  // Consecutive refreshes that could not reach the engine; sets the retry backoff
  const unreachableCountRef = useRef(0);

  const refreshTorrents = useCallback(async (): Promise<number> => {
    setTorrentsLoading(true);
    try {
      const response = await API.listTorrents({ withStats: true });
      setTorrents(response.torrents);
      setOtherError(null);
      setEngineUnreachable(null);
      if (unreachableCountRef.current > 0) {
        pushToast("success", "Connection to rustTorrent restored");
        unreachableCountRef.current = 0;
      }

      // Determine polling interval based on torrent states
      // Fast poll (1s) if any torrent is live/initializing, slow poll (5s) otherwise
      const hasActiveTorrents = response.torrents.some(
        (t) => t.stats?.state === "live" || t.stats?.state === "initializing",
      );
      return hasActiveTorrents ? 1000 : 5000;
    } catch (e) {
      const details = e as ApiErrorDetails;
      let delay = 5000;
      if (isEngineUnreachable(details)) {
        unreachableCountRef.current++;
        delay = retryDelayMs(unreachableCountRef.current);
        setEngineUnreachable(details);
      } else {
        setOtherError({ text: "Error refreshing torrents", details });
      }
      console.error(e);
      return delay;
    } finally {
      setTorrentsLoading(false);
    }
  }, [API, setTorrents, setTorrentsLoading, setOtherError, pushToast]);

  const setStats = useStatsStore((state) => state.setStats);

  // Register the refresh callback
  useEffect(() => {
    setRefreshTorrents(refreshTorrents);
  }, [refreshTorrents, setRefreshTorrents]);

  useEffect(() => {
    return customSetInterval(async () => refreshTorrents(), 0);
  }, [refreshTorrents]);

  useEffect(() => {
    return customSetInterval(
      async () =>
        API.stats().then(
          (stats) => {
            setStats(stats);
            return 1000;
          },
          (e) => {
            console.error(e);
            return 5000;
          },
        ),
      0,
    );
  }, [API, setStats]);

  // --- Drag and drop ---
  const [isDragging, setIsDragging] = useState(false);
  const dragCounterRef = useRef(0);

  const [pendingUpload, setPendingUpload] = useState<PendingUpload>(null);

  // When a single file or link is pending, list its files via the list_only API
  const singleUploadSource =
    pendingUpload?.type === "single"
      ? pendingUpload.file
      : pendingUpload?.type === "link"
        ? pendingUpload.link
        : null;
  const { listTorrentResponse, listTorrentLoading, listTorrentError } =
    useListOnlyUpload(singleUploadSource);

  // Pasting a magnet link or a .torrent URL anywhere starts the add flow. Text
  // typed into a field is left alone, so pasting still works there.
  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      if (
        e.target instanceof Element &&
        e.target.closest("input, textarea, [contenteditable='true']")
      ) {
        return;
      }
      const link = extractTorrentLink(e.clipboardData?.getData("text") ?? "");
      if (!link) return;
      e.preventDefault();
      setPendingUpload({ type: "link", link });
    };
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  }, []);

  // A magnet link opened from outside the page arrives as ?magnet=…, and the
  // page registers itself as the magnet handler so the browser can send them.
  useEffect(() => {
    const magnet = new URLSearchParams(window.location.search).get("magnet");
    if (magnet !== null) {
      window.history.replaceState(
        null,
        "",
        window.location.pathname + window.location.hash,
      );
      const link = extractTorrentLink(magnet);
      if (link) {
        setPendingUpload({ type: "link", link });
      } else {
        pushToast("error", "That magnet link is not valid");
      }
    }

    try {
      navigator.registerProtocolHandler?.(
        "magnet",
        `${window.location.origin}${window.location.pathname}?magnet=%s`,
      );
    } catch {
      // Refused on insecure origins or when the browser declines; paste and
      // drop still work without it
    }
  }, [pushToast]);

  const clearSingleUpload = () => setPendingUpload(null);

  const handleDragEnter = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragCounterRef.current++;
    if (dragCounterRef.current === 1) {
      setIsDragging(true);
    }
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragCounterRef.current--;
    if (dragCounterRef.current === 0) {
      setIsDragging(false);
    }
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
  }, []);

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      dragCounterRef.current = 0;
      setIsDragging(false);

      const droppedFiles = Array.from(e.dataTransfer.files);
      const files = droppedFiles.filter((f) => f.name.endsWith(".torrent"));

      if (files.length === 0) {
        // Dragged text or a link: a magnet or a .torrent URL starts the add flow
        const link = extractTorrentLink(
          e.dataTransfer.getData("text/uri-list") ||
            e.dataTransfer.getData("text/plain"),
        );
        if (link) {
          setPendingUpload({ type: "link", link });
        } else if (droppedFiles.length > 0) {
          pushToast("error", "Only .torrent files can be added");
        } else {
          pushToast("info", "Drop a .torrent file or a magnet link to add it");
        }
        return;
      }

      const skipped = droppedFiles.length - files.length;
      if (skipped > 0) {
        pushToast(
          "info",
          `Skipped ${skipped} ${skipped === 1 ? "file" : "files"} that ${skipped === 1 ? "is" : "are"} not .torrent`,
        );
      }

      if (files.length === 1) {
        setPendingUpload({ type: "single", file: files[0] });
      } else {
        setPendingUpload({ type: "multi", files });
      }
    },
    [pushToast],
  );

  // Handler for FileInput multi-file selection (via Toolbar)
  const handleMultiFileSelect = useCallback((files: File[]) => {
    if (files.length === 1) {
      setPendingUpload({ type: "single", file: files[0] });
    } else if (files.length > 1) {
      setPendingUpload({ type: "multi", files });
    }
  }, []);

  // Mobile navigation drawer: Escape closes it, focus moves into it when it
  // opens and returns to the opener when it closes
  const drawerOpen = !isLargeScreen && sidebarOpen;
  const drawerRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!drawerOpen) return;

    const opener = document.activeElement as HTMLElement | null;
    drawerRef.current?.querySelector<HTMLElement>("button")?.focus();

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setSidebarOpen(false);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      opener?.focus();
    };
  }, [drawerOpen, setSidebarOpen]);

  const pages: { key: CurrentPage; label: string }[] = [
    { key: "torrents", label: "Torrents" },
    ...(indexarrEnabled ? [{ key: "indexarr" as const, label: "Index" }] : []),
    { key: "rss", label: "RSS" },
  ];

  // The API base is the stream-logs URL without its path (null in mock mode)
  const apiBase =
    API.getStreamLogsUrl()?.replace(/\/stream_logs$/, "") ??
    window.location.origin;

  return (
    <div className="bg-surface h-dvh flex flex-col overflow-hidden">
      <h1 className="sr-only">rustTorrent</h1>
      <header>
        <Toolbar
          title={props.title}
          version={props.version}
          onMultiFileSelect={handleMultiFileSelect}
          onLogsClick={() => setLogsOpened(true)}
          menuButtons={props.menuButtons}
        />
      </header>

      <div className="flex-1 min-h-0 flex flex-row">
        {/* Sidebar - only on large screens, only on torrents page */}
        {isLargeScreen && currentPage === "torrents" && (
          <aside aria-label="Filters" className="flex shrink-0">
            <Sidebar />
          </aside>
        )}

        {/* Main content area */}
        <main
          className="flex-1 min-h-0 min-w-0 relative"
          onDragEnter={handleDragEnter}
          onDragLeave={handleDragLeave}
          onDragOver={handleDragOver}
          onDrop={handleDrop}
        >
          {engineUnreachable && !hasTorrentData ? (
            <EngineUnreachable
              apiBase={apiBase}
              details={engineUnreachable}
              onRetry={() => void refreshTorrents()}
            />
          ) : (
            <>
              {engineUnreachable && (
                <ConnectionLostBanner onRetry={() => void refreshTorrents()} />
              )}
              <RootContent />
            </>
          )}
          {isDragging && (
            <div className="absolute inset-0 z-50 flex items-center justify-center bg-surface/90">
              <div className="border-2 border-dashed border-primary rounded-lg p-8 text-center">
                <p className="text-lg font-semibold text-primary">
                  Drop .torrent files here
                </p>
              </div>
            </div>
          )}
        </main>
      </div>

      {/* Mobile navigation drawer */}
      {drawerOpen && (
        <div className="fixed inset-0 z-40">
          <div
            className="absolute inset-0 bg-black/50"
            onClick={() => setSidebarOpen(false)}
          />
          <div
            ref={drawerRef}
            role="dialog"
            aria-label="Navigation"
            className="absolute left-0 top-0 bottom-0 w-72 max-w-[85vw] bg-surface shadow-xl flex flex-col"
          >
            <div className="flex items-center gap-2 p-2 border-b border-divider">
              <div className="flex flex-1 gap-1">
                {pages.map((page) => (
                  <button
                    key={page.key}
                    type="button"
                    aria-pressed={currentPage === page.key}
                    onClick={() => {
                      setCurrentPage(page.key);
                      setSidebarOpen(false);
                    }}
                    className={`flex-1 px-2 py-1.5 rounded text-sm cursor-pointer transition-colors focus-ring touch-target ${
                      currentPage === page.key
                        ? "bg-primary/10 text-primary font-medium"
                        : "text-secondary hover:bg-surface-sunken hover:text-primary"
                    }`}
                  >
                    {page.label}
                  </button>
                ))}
              </div>
              <button
                type="button"
                aria-label="Close menu"
                title="Close menu"
                onClick={() => setSidebarOpen(false)}
                className="p-1.5 text-secondary hover:text-primary cursor-pointer rounded focus-ring touch-target"
              >
                <BsX className="w-5 h-5" />
              </button>
            </div>
            {currentPage === "torrents" && (
              <aside aria-label="Filters" className="flex-1 min-h-0 flex">
                <Sidebar variant="drawer" />
              </aside>
            )}
          </div>
        </div>
      )}

      <footer>
        <Footer />
      </footer>

      <LogStreamModal show={logsOpened} onClose={() => setLogsOpened(false)} />
      <AlertModal />
      <ToastViewport />

      {singleUploadSource !== null && (
        <FileSelectionModal
          onHide={clearSingleUpload}
          listTorrentResponse={listTorrentResponse}
          listTorrentError={listTorrentError}
          listTorrentLoading={listTorrentLoading}
          data={singleUploadSource}
        />
      )}

      {pendingUpload?.type === "multi" && (
        <MultiTorrentUploadModal
          files={pendingUpload.files}
          onHide={() => setPendingUpload(null)}
        />
      )}
    </div>
  );
};
