import { useContext, useEffect, useState } from "react";
import { TorrentListItem, WebSeedEntry } from "../../api-types";
import { APIContext } from "../../context";
import { Spinner } from "../Spinner";

interface HttpSourcesTabProps {
  torrent: TorrentListItem | null;
}

const headerCell =
  "px-2 py-1.5 text-left text-xs font-semibold text-tertiary uppercase tracking-wide whitespace-nowrap";

export const HttpSourcesTab: React.FC<HttpSourcesTabProps> = ({ torrent }) => {
  const API = useContext(APIContext);
  const [entries, setEntries] = useState<WebSeedEntry[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [unavailable, setUnavailable] = useState(false);

  const torrentId = torrent?.id;

  useEffect(() => {
    if (torrentId == null) return;
    let cancelled = false;
    setEntries(null);
    setLoading(true);
    setUnavailable(false);
    API.getWebSeeds(torrentId)
      .then((seeds) => {
        if (cancelled) return;
        setEntries(seeds);
        setLoading(false);
      })
      .catch(() => {
        if (cancelled) return;
        setLoading(false);
        setUnavailable(true);
      });
    return () => {
      cancelled = true;
    };
  }, [torrentId, API]);

  if (!torrent) return null;

  return (
    <div className="p-3 space-y-3">
      <p className="text-sm text-secondary">
        HTTP sources (web seeds) this torrent can download from.
      </p>

      {loading && (
        <div className="flex items-center gap-2 text-sm text-tertiary">
          <Spinner />
          <span>Loading HTTP sources...</span>
        </div>
      )}

      {unavailable && (
        <p className="text-sm text-tertiary">
          HTTP sources are not available from the API.
        </p>
      )}

      {!loading && entries && entries.length === 0 && (
        <p className="text-sm text-tertiary">No HTTP sources configured.</p>
      )}

      {!loading && entries && entries.length > 0 && (
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-divider">
                <th className={headerCell}>URL</th>
              </tr>
            </thead>
            <tbody>
              {entries.map((seed) => (
                <tr
                  key={seed.url}
                  className="border-b border-divider/50 hover:bg-surface-raised"
                >
                  <td
                    className="px-2 py-1 font-mono text-xs truncate max-w-[40rem]"
                    title={seed.url}
                  >
                    {seed.url}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
};
