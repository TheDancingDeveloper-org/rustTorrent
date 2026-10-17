const isMagnetUri = (token: string): boolean =>
  /^magnet:\?/i.test(token) && /[?&]xt=urn:bt(ih|mh):/i.test(token);

const isTorrentHttpUrl = (token: string): boolean => {
  try {
    const url = new URL(token);
    return (
      (url.protocol === "http:" || url.protocol === "https:") &&
      url.pathname.toLowerCase().endsWith(".torrent")
    );
  } catch {
    return false;
  }
};

/**
 * Finds the first torrent source in pasted or dropped text: a magnet URI, or an
 * HTTP(S) link to a .torrent file. Dropped text can be a text/uri-list with
 * several lines, so every whitespace-separated token is checked on its own.
 */
export function extractTorrentLink(text: string): string | null {
  for (const token of text.split(/\s+/)) {
    if (isMagnetUri(token) || isTorrentHttpUrl(token)) return token;
  }
  return null;
}
