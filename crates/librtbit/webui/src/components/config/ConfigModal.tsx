import React, { useContext, useEffect, useState } from "react";
import { TabbedConfigModal } from "../modal/TabbedConfigModal";
import { RateLimitsTab } from "./RateLimitsTab";
import { GeneralTab } from "./GeneralTab";
import { ConnectionTab } from "./ConnectionTab";
import { DHTTab } from "./DHTTab";
import { HttpApiTab } from "./HttpApiTab";
import { SecurityTab } from "./SecurityTab";
import { SeedingTab } from "./SeedingTab";
import { QueueTab } from "./QueueTab";
import { RSSTab } from "./RSSTab";
import { FoldersTab } from "./FoldersTab";
import { APIContext } from "../../context";
import { LimitsConfig, ErrorDetails } from "../../api-types";
import { ErrorWithLabel } from "../../rtbit-web";
import { Spinner } from "../Spinner";
import { Modal } from "../modal/Modal";
import { ModalBody } from "../modal/ModalBody";

export interface ConfigModalProps {
  isOpen: boolean;
  onClose: () => void;
  version?: string;
}

export const ConfigModal: React.FC<ConfigModalProps> = ({
  isOpen,
  onClose,
  version,
}) => {
  const [limits, setLimits] = useState<LimitsConfig>({
    upload_bps: null,
    download_bps: null,
    peer_limit: null,
    concurrent_init_limit: null,
    max_active_downloads: null,
    max_active_uploads: null,
    max_active_total: null,
  });
  const [rssHistoryLimit, setRssHistoryLimit] = useState<number | null>(500);
  const [downloadFolder, setDownloadFolder] = useState("");
  const [completedFolder, setCompletedFolder] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<ErrorWithLabel | null>(null);

  const API = useContext(APIContext);

  useEffect(() => {
    if (isOpen) {
      setLoading(true);
      setError(null);
      Promise.all([API.getLimits(), API.getFolders()])
        .then(([limitsConfig, foldersConfig]) => {
          setLimits(limitsConfig);
          setDownloadFolder(foldersConfig.download_folder);
          setCompletedFolder(foldersConfig.completed_folder ?? "");
        })
        .catch((e: ErrorDetails) => {
          setError({ text: "Error loading settings", details: e });
        })
        .finally(() => setLoading(false));
    }
  }, [isOpen, API]);

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      await Promise.all([
        API.setLimits(limits),
        API.setFolders({
          download_folder: downloadFolder,
          completed_folder: completedFolder || null,
        }),
      ]);
      onClose();
    } catch (e) {
      setError({ text: "Error saving settings", details: e as ErrorDetails });
    } finally {
      setSaving(false);
    }
  };

  if (loading && isOpen) {
    return (
      <Modal isOpen={isOpen} onClose={onClose} title="Settings">
        <ModalBody>
          <div className="flex justify-center p-4">
            <Spinner />
          </div>
        </ModalBody>
      </Modal>
    );
  }

  return (
    <TabbedConfigModal
      isOpen={isOpen}
      onClose={onClose}
      title="Settings"
      tabs={[
        {
          id: "general",
          label: "General",
          keywords: "version about theme dark light",
          content: <GeneralTab version={version ?? "unknown"} />,
        },
        {
          id: "folders",
          label: "Folders",
          keywords: "download completed directory path save",
          content: (
            <FoldersTab
              downloadFolder={downloadFolder}
              completedFolder={completedFolder}
              onDownloadFolderChange={setDownloadFolder}
              onCompletedFolderChange={setCompletedFolder}
            />
          ),
        },
        {
          id: "speed",
          label: "Speed",
          keywords: "rate limit upload download bandwidth peers concurrent",
          content: (
            <RateLimitsTab
              downloadBps={limits.download_bps}
              uploadBps={limits.upload_bps}
              peerLimit={limits.peer_limit}
              concurrentInitLimit={limits.concurrent_init_limit}
              onDownloadBpsChange={(v) =>
                setLimits((l) => ({ ...l, download_bps: v }))
              }
              onUploadBpsChange={(v) =>
                setLimits((l) => ({ ...l, upload_bps: v }))
              }
              onPeerLimitChange={(v) =>
                setLimits((l) => ({ ...l, peer_limit: v }))
              }
              onConcurrentInitLimitChange={(v) =>
                setLimits((l) => ({ ...l, concurrent_init_limit: v }))
              }
            />
          ),
        },
        {
          id: "connections",
          label: "Connections",
          keywords: "port listen peers tcp utp incoming",
          content: <ConnectionTab />,
        },
        {
          id: "queue",
          label: "Queue",
          keywords: "queue active downloads uploads limit concurrent",
          content: (
            <QueueTab
              maxActiveDownloads={limits.max_active_downloads}
              maxActiveUploads={limits.max_active_uploads}
              maxActiveTotal={limits.max_active_total}
              onMaxActiveDownloadsChange={(v) =>
                setLimits((l) => ({ ...l, max_active_downloads: v }))
              }
              onMaxActiveUploadsChange={(v) =>
                setLimits((l) => ({ ...l, max_active_uploads: v }))
              }
              onMaxActiveTotalChange={(v) =>
                setLimits((l) => ({ ...l, max_active_total: v }))
              }
            />
          ),
        },
        {
          id: "seeding",
          label: "Seeding",
          keywords: "seed ratio share limit stop",
          content: <SeedingTab />,
        },
        {
          id: "dht",
          label: "DHT",
          keywords: "dht bootstrap nodes peer discovery",
          content: <DHTTab />,
        },
        {
          id: "security",
          label: "Security",
          keywords: "password auth login credentials",
          content: <SecurityTab />,
        },
        {
          id: "http-api",
          label: "HTTP API",
          keywords: "http api web ui cors bind address port",
          content: <HttpApiTab />,
        },
        {
          id: "rss",
          label: "RSS",
          keywords: "rss feed filter history",
          content: (
            <RSSTab
              rssHistoryLimit={rssHistoryLimit}
              onRssHistoryLimitChange={setRssHistoryLimit}
            />
          ),
        },
      ]}
      onSave={handleSave}
      isSaving={saving}
      error={error}
      showResetButton={false}
    />
  );
};
