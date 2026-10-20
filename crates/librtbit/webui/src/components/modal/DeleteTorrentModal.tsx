import { useContext, useState } from "react";
import { TorrentListItem } from "../../api-types";
import { APIContext } from "../../context";
import { ErrorWithLabel } from "../../rtbit-web";
import { useTorrentStore } from "../../stores/torrentStore";
import { useUIStore } from "../../stores/uiStore";
import { Button } from "../buttons/Button";
import { ErrorComponent } from "../ErrorComponent";
import { Spinner } from "../Spinner";
import { Modal } from "./Modal";
import { ModalBody } from "./ModalBody";
import { ModalFooter } from "./ModalFooter";

export const DeleteTorrentModal: React.FC<{
  show: boolean;
  onHide: () => void;
  torrents: Pick<TorrentListItem, "id" | "name">[];
}> = ({ show, onHide, torrents }) => {
  const [deleteFiles, setDeleteFiles] = useState(false);
  const [error, setError] = useState<ErrorWithLabel | null>(null);
  const [deleting, setDeleting] = useState(false);

  const API = useContext(APIContext);
  const refreshTorrents = useTorrentStore((state) => state.refreshTorrents);
  const clearSelection = useUIStore((state) => state.clearSelection);

  if (!show || torrents.length === 0) {
    return null;
  }

  const close = () => {
    setDeleteFiles(false);
    setError(null);
    setDeleting(false);
    onHide();
  };

  const deleteTorrents = async () => {
    setDeleting(true);
    setError(null);

    const deleteMethod = deleteFiles ? API.delete : API.forget;
    const errors: string[] = [];

    for (const torrent of torrents) {
      try {
        await deleteMethod(torrent.id);
      } catch (e) {
        const name = torrent.name || `id=${torrent.id}`;
        errors.push(`${name}: ${e}`);
      }
    }

    if (errors.length > 0) {
      setError({
        text: `Failed to delete ${errors.length} torrent${errors.length > 1 ? "s" : ""}`,
        details: { text: errors.join("\n") },
      });
      setDeleting(false);
    } else {
      clearSelection();
      refreshTorrents();
      close();
    }
  };

  const isBulk = torrents.length > 1;
  const title = isBulk
    ? `Delete ${torrents.length} torrents`
    : "Delete torrent";

  return (
    <Modal isOpen={show} onClose={onHide} title={title}>
      <ModalBody>
        <p className="text-text mb-3">
          {isBulk
            ? "Are you sure you want to delete the following torrents?"
            : "Are you sure you want to delete this torrent?"}
        </p>

        <div
          className={`rounded-md bg-surface-sunken p-3 ${
            isBulk ? "max-h-48 overflow-y-auto" : ""
          }`}
        >
          <ul className="space-y-1">
            {torrents.map((torrent) => (
              <li
                key={torrent.id}
                className="text-text truncate"
                title={torrent.name ?? undefined}
              >
                <span className="font-medium">
                  {torrent.name || `Torrent #${torrent.id}`}
                </span>
              </li>
            ))}
          </ul>
        </div>

        <div className="mt-4 flex items-center">
          <input
            type="checkbox"
            id="deleteFiles"
            className="h-4 w-4 rounded border-divider accent-primary focus-ring"
            onChange={() => setDeleteFiles(!deleteFiles)}
            checked={deleteFiles}
          />
          <label
            htmlFor="deleteFiles"
            className="ml-2 text-text"
          >
            Also delete downloaded files
          </label>
        </div>

        {error && <ErrorComponent error={error} />}
      </ModalBody>

      <ModalFooter>
        {deleting && <Spinner />}
        <Button variant="cancel" onClick={close}>
          Cancel
        </Button>
        <Button variant="danger" onClick={deleteTorrents} disabled={deleting}>
          {isBulk ? `Delete ${torrents.length} Torrents` : "Delete Torrent"}
        </Button>
      </ModalFooter>
    </Modal>
  );
};
