import { useContext, useState } from "react";
import { TorrentListItem, TorrentStats } from "../../api-types";
import { APIContext } from "../../context";
import { IconButton } from "./IconButton";
import { DeleteTorrentModal } from "../modal/DeleteTorrentModal";
import { FaCog, FaPause, FaPlay, FaTrash } from "react-icons/fa";
import { useErrorStore } from "../../stores/errorStore";
import { useTorrentStore } from "../../stores/torrentStore";
import { useUIStore } from "../../stores/uiStore";

export const TorrentActions: React.FC<{
  torrent: TorrentListItem & { stats: TorrentStats };
}> = ({ torrent }) => {
  const { id, name, stats } = torrent;
  const state = stats.state;

  const [disabled, setDisabled] = useState<boolean>(false);
  const [deleting, setDeleting] = useState<boolean>(false);

  const refreshTorrents = useTorrentStore((state) => state.refreshTorrents);
  const openDetailsModal = useUIStore((state) => state.openDetailsModal);

  const canPause = state == "live";
  const canUnpause = state == "paused" || state == "error";
  const canConfigure = state == "paused" || state == "live";

  const setCloseableError = useErrorStore((state) => state.setCloseableError);

  const API = useContext(APIContext);

  const unpause = () => {
    setDisabled(true);
    API.start(id)
      .then(
        () => {
          refreshTorrents();
        },
        (e) => {
          setCloseableError({
            text: `Error starting torrent id=${id}`,
            details: e,
          });
        },
      )
      .finally(() => setDisabled(false));
  };

  const pause = () => {
    setDisabled(true);
    API.pause(id)
      .then(
        () => {
          refreshTorrents();
        },
        (e) => {
          setCloseableError({
            text: `Error pausing torrent id=${id}`,
            details: e,
          });
        },
      )
      .finally(() => setDisabled(false));
  };

  const startDeleting = () => {
    setDisabled(true);
    setDeleting(true);
  };

  const cancelDeleting = () => {
    setDisabled(false);
    setDeleting(false);
  };

  return (
    <div className="flex w-full justify-center gap-1 sm:gap-2">
      {canUnpause && (
        <IconButton
          label="Resume"
          color="success"
          onClick={unpause}
          disabled={disabled}
        >
          <FaPlay />
        </IconButton>
      )}
      {canPause && (
        <IconButton
          label="Pause"
          color="secondary"
          onClick={pause}
          disabled={disabled}
        >
          <FaPause />
        </IconButton>
      )}
      {canConfigure && (
        <IconButton
          label="Torrent settings"
          color="secondary"
          onClick={() => openDetailsModal(id)}
          disabled={disabled}
        >
          <FaCog />
        </IconButton>
      )}
      <IconButton
        label="Delete"
        color="error"
        onClick={startDeleting}
        disabled={disabled}
      >
        <FaTrash />
      </IconButton>
      <DeleteTorrentModal
        show={deleting}
        onHide={cancelDeleting}
        torrents={[{ id, name }]}
      />
    </div>
  );
};
