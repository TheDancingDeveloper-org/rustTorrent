// Turns two snapshots of the torrent list into short messages for the live
// region: what was added, removed, paused, resumed or failed since last time.

export interface TorrentSnapshot {
  name: string;
  state: string | undefined;
  error: string | null | undefined;
}

const MAX_MESSAGES = 3;

export function describeTorrentChanges(
  prev: ReadonlyMap<number, TorrentSnapshot>,
  next: ReadonlyMap<number, TorrentSnapshot>,
): string[] {
  const messages: string[] = [];

  for (const [id, cur] of next) {
    const before = prev.get(id);
    if (!before) {
      messages.push(`Added ${cur.name}`);
      continue;
    }
    if (before.state === cur.state) continue;
    if (cur.state === "paused") {
      messages.push(`Paused ${cur.name}`);
    } else if (cur.state === "live") {
      messages.push(`Resumed ${cur.name}`);
    } else if (cur.state === "error") {
      messages.push(
        cur.error
          ? `Error on ${cur.name}: ${cur.error}`
          : `Error on ${cur.name}`,
      );
    }
  }

  for (const [id, before] of prev) {
    if (!next.has(id)) messages.push(`Removed ${before.name}`);
  }

  if (messages.length > MAX_MESSAGES) {
    return [`${messages.length} torrent changes`];
  }
  return messages;
}
