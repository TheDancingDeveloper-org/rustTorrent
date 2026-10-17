import { useContext, useEffect, useState } from "react";
import {
  AddTorrentResponse,
  ErrorDetails as ApiErrorDetails,
} from "../api-types";
import { APIContext } from "../context";
import { ErrorWithLabel } from "../rtbit-web";

/**
 * Lists the files of a torrent (a .torrent file or a magnet URI) with the
 * list_only upload call. A result for a previous `data` value is dropped once
 * `data` changes, and passing null resets the state.
 */
export function useListOnlyUpload(data: string | File | null) {
  const API = useContext(APIContext);
  const [listTorrentResponse, setListTorrentResponse] =
    useState<AddTorrentResponse | null>(null);
  const [listTorrentLoading, setListTorrentLoading] = useState(false);
  const [listTorrentError, setListTorrentError] =
    useState<ErrorWithLabel | null>(null);

  useEffect(() => {
    setListTorrentResponse(null);
    setListTorrentError(null);
    if (data === null) {
      setListTorrentLoading(false);
      return;
    }

    let cancelled = false;
    setListTorrentLoading(true);
    const t = setTimeout(async () => {
      try {
        const response = await API.uploadTorrent(data, { list_only: true });
        if (!cancelled) setListTorrentResponse(response);
      } catch (e) {
        if (!cancelled)
          setListTorrentError({
            text: "Error listing torrent files",
            details: e as ApiErrorDetails,
          });
      } finally {
        if (!cancelled) setListTorrentLoading(false);
      }
    }, 0);

    return () => {
      cancelled = true;
      clearTimeout(t);
    };
  }, [API, data]);

  return { listTorrentResponse, listTorrentLoading, listTorrentError };
}
