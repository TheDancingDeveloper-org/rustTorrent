import { useEffect, useRef } from "react";
import { useUIStore } from "../stores/uiStore";
import { useTorrentStore } from "../stores/torrentStore";
import { useErrorStore } from "../stores/errorStore";
import {
  describeTorrentChanges,
  type TorrentSnapshot,
} from "../helper/torrentChanges";

/**
 * Feeds the live region with state changes: torrents added, paused, resumed,
 * removed or failed, and errors shown to the user. Mount once.
 */
export function useAnnouncements() {
  const torrents = useTorrentStore((state) => state.torrents);
  const closeableError = useErrorStore((state) => state.closeableError);
  const announce = useUIStore((state) => state.announce);
  const previous = useRef<Map<number, TorrentSnapshot> | null>(null);

  useEffect(() => {
    if (!torrents) return;
    const next = new Map<number, TorrentSnapshot>(
      torrents.map((t) => [
        t.id,
        {
          name: t.name ?? `torrent ${t.id}`,
          state: t.stats?.state,
          error: t.stats?.error,
        },
      ]),
    );
    const prev = previous.current;
    previous.current = next;
    // The first snapshot is the existing list, not news
    if (!prev) return;
    const messages = describeTorrentChanges(prev, next);
    if (messages.length > 0) announce(messages.join(". "));
  }, [torrents, announce]);

  useEffect(() => {
    if (closeableError) announce(`Error: ${closeableError.text}`);
  }, [closeableError, announce]);
}
