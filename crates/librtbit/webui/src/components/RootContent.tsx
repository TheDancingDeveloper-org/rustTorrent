import { Suspense, lazy } from "react";
import { CardLayout } from "./CardLayout";
import { ErrorComponent } from "./ErrorComponent";
import { useTorrentStore } from "../stores/torrentStore";
import { useErrorStore } from "../stores/errorStore";
import { useUIStore } from "../stores/uiStore";
import { useIsLargeScreen } from "../hooks/useIsLargeScreen";
import { CompactLayout } from "./compact/CompactLayout";
import { Spinner } from "./Spinner";

// Feature pages are loaded on first visit so they stay out of the main chunk.
const IndexarrBrowse = lazy(() =>
  import("./IndexarrBrowse").then((m) => ({ default: m.IndexarrBrowse })),
);
const RSSPage = lazy(() =>
  import("./RSSPage").then((m) => ({ default: m.RSSPage })),
);
const StatisticsPage = lazy(() =>
  import("./StatisticsPage").then((m) => ({ default: m.StatisticsPage })),
);

export const RootContent = () => {
  const closeableError = useErrorStore((state) => state.closeableError);
  const setCloseableError = useErrorStore((state) => state.setCloseableError);
  const otherError = useErrorStore((state) => state.otherError);
  const torrents = useTorrentStore((state) => state.torrents);
  const torrentsInitiallyLoading = useTorrentStore(
    (state) => state.torrentsInitiallyLoading,
  );

  const viewMode = useUIStore((state) => state.viewMode);
  const currentPage = useUIStore((state) => state.currentPage);
  const isLargeScreen = useIsLargeScreen();

  const useCompactLayout = viewMode === "compact" && isLargeScreen;

  if (currentPage === "indexarr") {
    return (
      <div className="h-full flex flex-col">
        <ErrorComponent
          error={closeableError}
          remove={() => setCloseableError(null)}
        />
        <ErrorComponent error={otherError} />
        <div className="flex-1 min-h-0">
          <Suspense fallback={<Spinner />}>
            <IndexarrBrowse />
          </Suspense>
        </div>
      </div>
    );
  }

  if (currentPage === "statistics") {
    return (
      <div className="h-full flex flex-col">
        <ErrorComponent
          error={closeableError}
          remove={() => setCloseableError(null)}
        />
        <ErrorComponent error={otherError} />
        <div className="flex-1 min-h-0">
          <Suspense fallback={<Spinner />}>
            <StatisticsPage />
          </Suspense>
        </div>
      </div>
    );
  }

  if (currentPage === "rss") {
    return (
      <div className="h-full flex flex-col">
        <ErrorComponent
          error={closeableError}
          remove={() => setCloseableError(null)}
        />
        <ErrorComponent error={otherError} />
        <div className="flex-1 min-h-0">
          <Suspense fallback={<Spinner />}>
            <RSSPage />
          </Suspense>
        </div>
      </div>
    );
  }

  return (
    <div className={useCompactLayout ? "h-full" : "h-full flex flex-col"}>
      <ErrorComponent
        error={closeableError}
        remove={() => setCloseableError(null)}
      />
      <ErrorComponent error={otherError} />
      {useCompactLayout ? (
        <CompactLayout torrents={torrents} loading={torrentsInitiallyLoading} />
      ) : (
        <div className="flex-1 min-h-0">
          <CardLayout torrents={torrents} loading={torrentsInitiallyLoading} />
        </div>
      )}
    </div>
  );
};
