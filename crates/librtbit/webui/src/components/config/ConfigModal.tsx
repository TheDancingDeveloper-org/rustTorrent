import React, { useContext, useEffect, useState } from "react";
import { TabbedConfigModal } from "../modal/TabbedConfigModal";
import { RateLimitsTab } from "./RateLimitsTab";
import { GeneralTab } from "./GeneralTab";
import { ConnectionTab } from "./ConnectionTab";
import { DHTTab } from "./DHTTab";
import { HttpApiTab } from "./HttpApiTab";
import { SecurityTab } from "./SecurityTab";
import { BehaviourTab } from "./BehaviourTab";
import { RSSTab } from "./RSSTab";
import { FoldersTab } from "./FoldersTab";
import { DownloadsTab } from "./DownloadsTab";
import { AdvancedTab } from "./AdvancedTab";
import { ProxyFilterTab } from "./ProxyFilterTab";
import { SETTINGS } from "./settingsRegistry";
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
      settings={SETTINGS}
      tabs={[
        {
          id: "general",
          label: "General",
          content: <GeneralTab version={version ?? "unknown"} />,
        },
        {
          id: "folders",
          label: "Folders",
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
          id: "downloads",
          label: "Downloads",
          content: <DownloadsTab />,
        },
        {
          id: "speed",
          label: "Speed",
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
          content: <ConnectionTab />,
        },
        {
          id: "behaviour",
          label: "Behaviour",
          content: (
            <BehaviourTab
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
          id: "dht",
          label: "DHT",
          content: <DHTTab />,
        },
        {
          id: "security",
          label: "Security",
          content: <SecurityTab />,
        },
        {
          id: "http-api",
          label: "HTTP API",
          content: <HttpApiTab />,
        },
        {
          id: "connection",
          label: "Connection",
          content: <ProxyFilterTab />,
        },
        {
          id: "advanced",
          label: "Advanced",
          content: <AdvancedTab />,
        },
        {
          id: "rss",
          label: "RSS",
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
