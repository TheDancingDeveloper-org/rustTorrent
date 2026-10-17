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

export interface ErrorWithLabel {
  text: string;
  details?: ApiErrorDetails;
}

export interface ContextType {
  setCloseableError: (error: ErrorWithLabel | null) => void;
  refreshTorrents: () => void;
}

type PendingUpload =
  { type: "single"; file: File } | { type: "multi"; files: File[] } | null;

const isEngineUnreachable = (e: ApiErrorDetails): boolean =>
  e.status === undefined
    ? e.text === "network error"
    : e.status === 404 || e.status >= 500;

export const RtbitWebUI = (props: {
  title: string;
  version: string;
  menuButtons?: JSX.Element[];
}) => {
  const [logsOpened, setLogsOpened] = useState<boolean>(false);
  const setOtherError = useErrorStore((state) => state.setOtherError);

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

  // The engine is unreachable when the request got no response, or the server
  // answered 404 or 5xx. Other failures keep the normal error alert.
  const [engineUnreachable, setEngineUnreachable] =
    useState<ApiErrorDetails | null>(null);

  const refreshTorrents = useCallback(async (): Promise<number> => {
    setTorrentsLoading(true);
    try {
      const response = await API.listTorrents({ withStats: true });
      setTorrents(response.torrents);
      setOtherError(null);
      setEngineUnreachable(null);

      // Determine polling interval based on torrent states
      // Fast poll (1s) if any torrent is live/initializing, slow poll (5s) otherwise
      const hasActiveTorrents = response.torrents.some(
        (t) => t.stats?.state === "live" || t.stats?.state === "initializing",
      );
      return hasActiveTorrents ? 1000 : 5000;
    } catch (e) {
      const details = e as ApiErrorDetails;
      if (isEngineUnreachable(details)) {
        setEngineUnreachable(details);
      } else {
        setOtherError({ text: "Error refreshing torrents", details });
      }
      console.error(e);
      return 5000;
    } finally {
      setTorrentsLoading(false);
    }
  }, [API, setTorrents, setTorrentsLoading, setOtherError]);

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

  // When a single file is pending, list its files via the list_only API
  const singleUploadFile =
    pendingUpload?.type === "single" ? pendingUpload.file : null;
  const { listTorrentResponse, listTorrentLoading, listTorrentError } =
    useListOnlyUpload(singleUploadFile);

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

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragCounterRef.current = 0;
    setIsDragging(false);

    const files = Array.from(e.dataTransfer.files).filter((f) =>
      f.name.endsWith(".torrent"),
    );

    if (files.length === 0) return;

    if (files.length === 1) {
      setPendingUpload({ type: "single", file: files[0] });
    } else {
      setPendingUpload({ type: "multi", files });
    }
  }, []);

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
          className="flex-1 min-h-0 relative"
          onDragEnter={handleDragEnter}
          onDragLeave={handleDragLeave}
          onDragOver={handleDragOver}
          onDrop={handleDrop}
        >
          {engineUnreachable ? (
            <EngineUnreachable
              apiBase={apiBase}
              details={engineUnreachable}
              onRetry={() => void refreshTorrents()}
            />
          ) : (
            <RootContent />
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

      {pendingUpload?.type === "single" && (
        <FileSelectionModal
          onHide={clearSingleUpload}
          listTorrentResponse={listTorrentResponse}
          listTorrentError={listTorrentError}
          listTorrentLoading={listTorrentLoading}
          data={pendingUpload.file}
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
