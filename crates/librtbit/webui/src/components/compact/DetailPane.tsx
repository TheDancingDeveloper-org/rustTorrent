import { useContext, useEffect, useMemo, useState } from "react";
import { APIContext } from "../../context";
import { loopUntilSuccess } from "../../helper/loopUntilSuccess";
import { useUIStore } from "../../stores/uiStore";
import { useTorrentStore } from "../../stores/torrentStore";
import { OverviewTab } from "./OverviewTab";
import { FilesTab } from "./FilesTab";
import { PeersTab } from "./PeersTab";
import { TrackersTab } from "./TrackersTab";
import { SpeedTab } from "./SpeedTab";
import { TabButton, TabList } from "../Tabs";
import { LogStream } from "../LogStream";
import { loadViewPref, oneOf, saveViewPref } from "../../helper/viewPrefs";

type TabId = "overview" | "files" | "peers" | "trackers" | "speed" | "logs";

const VALID_TABS: TabId[] = [
  "overview",
  "files",
  "peers",
  "trackers",
  "speed",
  "logs",
];

export const DetailPane: React.FC = () => {
  const selectedTorrentIds = useUIStore((state) => state.selectedTorrentIds);
  const detailPaneRequestedTab = useUIStore(
    (state) => state.detailPaneRequestedTab,
  );
  const setDetailPaneTab = useUIStore((state) => state.setDetailPaneTab);
  const [activeTab, setActiveTab] = useState<TabId>(() =>
    loadViewPref<TabId>("detailPaneTab", "overview", oneOf(VALID_TABS)),
  );
  useEffect(() => {
    saveViewPref("detailPaneTab", activeTab);
  }, [activeTab]);

  // Respond to programmatic tab switch requests
  useEffect(() => {
    if (
      detailPaneRequestedTab &&
      VALID_TABS.includes(detailPaneRequestedTab as TabId)
    ) {
      setActiveTab(detailPaneRequestedTab as TabId);
      setDetailPaneTab(null);
    }
  }, [detailPaneRequestedTab, setDetailPaneTab]);

  const selectedArray = Array.from(selectedTorrentIds);
  const selectedCount = selectedArray.length;

  if (selectedCount === 0) {
    return (
      <div className="h-full border-t border-divider bg-surface-raised flex items-center justify-center">
        <p className="text-tertiary">Select a torrent to view details</p>
      </div>
    );
  }

  if (selectedCount > 1) {
    return (
      <div className="h-full border-t border-divider bg-surface-raised flex items-center justify-center">
        <p className="text-tertiary">{selectedCount} torrents selected</p>
      </div>
    );
  }

  const selectedId = selectedArray[0];

  return (
    <div className="h-full border-t border-divider flex flex-col bg-surface">
      <TabList className="bg-surface-raised">
        <TabButton
          id="overview"
          label="Overview"
          active={activeTab === "overview"}
          onClick={() => setActiveTab("overview")}
        />
        <TabButton
          id="files"
          label="Files"
          active={activeTab === "files"}
          onClick={() => setActiveTab("files")}
        />
        <TabButton
          id="peers"
          label="Peers"
          active={activeTab === "peers"}
          onClick={() => setActiveTab("peers")}
        />
        <TabButton
          id="trackers"
          label="Trackers"
          active={activeTab === "trackers"}
          onClick={() => setActiveTab("trackers")}
        />
        <TabButton
          id="speed"
          label="Speed"
          active={activeTab === "speed"}
          onClick={() => setActiveTab("speed")}
        />
        <TabButton
          id="logs"
          label="Logs"
          active={activeTab === "logs"}
          onClick={() => setActiveTab("logs")}
        />
      </TabList>
      <div className="flex-1 min-h-0 overflow-auto">
        <DetailPaneContent torrentId={selectedId} activeTab={activeTab} />
      </div>
    </div>
  );
};

interface DetailPaneContentProps {
  torrentId: number;
  activeTab: TabId;
}

const DetailPaneContent: React.FC<DetailPaneContentProps> = ({
  torrentId,
  activeTab,
}) => {
  const API = useContext(APIContext);
  const [fetchDetails, setFetchDetails] = useState(false);

  // Get torrent and details from store
  const torrent = useTorrentStore((state) =>
    state.torrents?.find((t) => t.id === torrentId),
  );
  const cachedDetails = useTorrentStore((state) => state.getDetails(torrentId));
  const setDetails = useTorrentStore((state) => state.setDetails);
  const refreshTorrents = useTorrentStore((state) => state.refreshTorrents);

  // Trigger fetch when Files tab is active and not cached
  useEffect(() => {
    if (activeTab !== "files") return;
    if (cachedDetails) return;
    setFetchDetails(true);
  }, [activeTab, torrentId, cachedDetails]);

  // Fetch details when requested
  useEffect(() => {
    if (!fetchDetails) return;
    return loopUntilSuccess(async () => {
      const details = await API.getTorrentDetails(torrentId);
      setDetails(torrentId, details);
      setFetchDetails(false);
    }, 1000);
  }, [fetchDetails, torrentId]);

  const forceRefreshCallback = () => {
    refreshTorrents();
    setFetchDetails(true);
  };

  const statsResponse = torrent?.stats ?? null;

  const logsUrl = API.getStreamLogsUrl();
  const torrentInfoHash = torrent?.info_hash ?? "";
  const logFilter = useMemo(
    () => ({ id: torrentId, infoHash: torrentInfoHash }),
    [torrentId, torrentInfoHash],
  );

  return (
    <>
      {activeTab === "overview" && <OverviewTab torrent={torrent ?? null} />}
      {activeTab === "files" && (
        <FilesTab
          torrentId={torrentId}
          detailsResponse={cachedDetails}
          statsResponse={statsResponse}
          onRefresh={forceRefreshCallback}
        />
      )}
      {activeTab === "peers" && <PeersTab torrent={torrent ?? null} />}
      {activeTab === "trackers" && <TrackersTab torrent={torrent ?? null} />}
      {activeTab === "speed" && <SpeedTab torrent={torrent ?? null} />}
      {activeTab === "logs" && (
        <div className="h-full p-2">
          {logsUrl ? (
            <LogStream url={logsUrl} maxLines={500} torrentFilter={logFilter} />
          ) : (
            <p className="text-tertiary p-3">Log streaming not available</p>
          )}
        </div>
      )}
    </>
  );
};
