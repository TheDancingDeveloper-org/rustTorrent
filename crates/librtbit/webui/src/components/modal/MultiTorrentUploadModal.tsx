import { useContext, useEffect, useState } from "react";
import { APIContext } from "../../context";
import { useTorrentStore } from "../../stores/torrentStore";
import { useUIStore } from "../../stores/uiStore";
import { CategoryInfo } from "../../api-types";
import { Modal } from "./Modal";
import { ModalBody } from "./ModalBody";
import { ModalFooter } from "./ModalFooter";
import { Button } from "../buttons/Button";
import { FormInput } from "../forms/FormInput";
import { FormCheckbox } from "../forms/FormCheckbox";
import { Form } from "../forms/Form";
import { Spinner } from "../Spinner";
import { loadStartPaused } from "../../helper/behaviourPrefs";
import { BsCheckCircleFill, BsXCircleFill } from "react-icons/bs";

type FileStatus = "pending" | "uploading" | "success" | "error";

interface FileEntry {
  file: File;
  status: FileStatus;
  error?: string;
}

export const MultiTorrentUploadModal = ({
  files,
  onHide,
}: {
  files: File[];
  onHide: () => void;
}) => {
  const API = useContext(APIContext);
  const refreshTorrents = useTorrentStore((state) => state.refreshTorrents);

  const categories = useUIStore((state) => state.categories);
  const setCategories = useUIStore((state) => state.setCategories);

  const [outputFolder, setOutputFolder] = useState("");
  const [selectedCategory, setSelectedCategory] = useState("");
  const [startTorrent, setStartTorrent] = useState(() => !loadStartPaused());
  const [entries, setEntries] = useState<FileEntry[]>(
    files.map((file) => ({ file, status: "pending" })),
  );
  const [uploading, setUploading] = useState(false);
  const [done, setDone] = useState(false);

  // Fetch categories when modal opens
  useEffect(() => {
    API.getCategories()
      .then((cats) => setCategories(cats))
      .catch(() => {});
  }, [API, setCategories]);

  const categoryNames = Object.keys(categories).sort((a, b) =>
    a.localeCompare(b),
  );

  const handleUploadAll = async () => {
    setUploading(true);
    const updated = [...entries];

    for (let i = 0; i < updated.length; i++) {
      updated[i] = { ...updated[i], status: "uploading" };
      setEntries([...updated]);

      try {
        await API.uploadTorrent(updated[i].file, {
          overwrite: true,
          output_folder: outputFolder,
          paused: !startTorrent,
          category: selectedCategory || undefined,
        });
        updated[i] = { ...updated[i], status: "success" };
      } catch (e: any) {
        updated[i] = {
          ...updated[i],
          status: "error",
          error: e?.text || e?.message || "Upload failed",
        };
      }
      setEntries([...updated]);
    }

    setUploading(false);
    setDone(true);
    refreshTorrents();
  };

  const statusIcon = (status: FileStatus) => {
    switch (status) {
      case "uploading":
        return <Spinner className="w-4 h-4 inline-block" />;
      case "success":
        return <BsCheckCircleFill className="text-success inline-block" />;
      case "error":
        return <BsXCircleFill className="text-error inline-block" />;
      default:
        return <span className="text-secondary">--</span>;
    }
  };

  return (
    <Modal isOpen={true} onClose={onHide} title="Upload Multiple Torrents">
      <ModalBody>
        <Form>
          <FormInput
            label="Output folder"
            name="multi_output_folder"
            inputType="text"
            placeholder="Server default"
            value={outputFolder}
            onChange={(e) => setOutputFolder(e.target.value)}
          />
          {categoryNames.length > 0 && (
            <div className="flex flex-col gap-1">
              <label
                htmlFor="multi_category"
                className="text-sm font-medium text-secondary"
              >
                Category
              </label>
              <select
                id="multi_category"
                value={selectedCategory}
                onChange={(e) => setSelectedCategory(e.target.value)}
                className="px-2 py-1.5 text-sm bg-surface border border-divider rounded focus-ring focus:border-primary"
              >
                <option value="">None</option>
                {categoryNames.map((name) => (
                  <option key={name} value={name}>
                    {name}
                  </option>
                ))}
              </select>
            </div>
          )}
          <FormCheckbox
            label="Start torrents after adding"
            checked={startTorrent}
            onChange={() => setStartTorrent(!startTorrent)}
            name="start_torrent"
          />
        </Form>

        <div className="mt-3 max-h-60 overflow-y-auto border border-divider rounded">
          {entries.map((entry, idx) => (
            <div
              key={idx}
              className="flex items-center gap-2 px-3 py-1.5 border-b border-divider last:border-b-0"
            >
              <span className="w-5 flex-shrink-0">
                {statusIcon(entry.status)}
              </span>
              <span className="truncate min-w-0 flex-1">{entry.file.name}</span>
              {entry.error && (
                <span className="text-error text-xs truncate max-w-[200px]">
                  {entry.error}
                </span>
              )}
            </div>
          ))}
        </div>
      </ModalBody>
      <ModalFooter>
        {uploading && <Spinner />}
        <Button onClick={onHide} variant="cancel">
          {done ? "Close" : "Cancel"}
        </Button>
        {!done && (
          <Button
            onClick={handleUploadAll}
            variant="primary"
            disabled={uploading}
          >
            Upload All ({entries.length})
          </Button>
        )}
      </ModalFooter>
    </Modal>
  );
};
