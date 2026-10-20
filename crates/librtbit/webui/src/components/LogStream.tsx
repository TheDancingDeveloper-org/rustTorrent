import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { ErrorWithLabel } from "../rtbit-web";
import { ErrorComponent } from "./ErrorComponent";
import { loopUntilSuccess } from "../helper/loopUntilSuccess";
import debounce from "lodash.debounce";
import { LogLine } from "./LogLine";
import { JSONLogLine } from "../api-types";
import { Virtuoso } from "react-virtuoso";

interface LogStreamProps {
  url: string;
  maxLines?: number;
  /** When set, only show lines belonging to this torrent (matched via
   * tracing spans `torrent{id=...}` or an `info_hash` span/field). */
  torrentFilter?: { id: number; infoHash: string };
}

const lineMatchesTorrent = (
  line: JSONLogLine,
  content: string,
  filter: { id: number; infoHash: string },
): boolean => {
  const spans = [...(line.spans ?? []), ...(line.span ? [line.span] : [])];
  for (const span of spans) {
    if (span.name === "torrent" && Number(span.id) === filter.id) {
      return true;
    }
  }
  // Tracker/DHT/magnet spans carry info_hash rather than the torrent id
  return (
    filter.infoHash.length > 0 &&
    content.toLowerCase().includes(filter.infoHash.toLowerCase())
  );
};

export interface Line {
  id: number;
  content: string;
  parsed: JSONLogLine;
  show: boolean;
}

const LOG_LEVELS = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"] as const;

const levelButtonActiveClass = (level: string): string => {
  switch (level) {
    case "TRACE":
      return "bg-surface-sunken text-text";
    case "DEBUG":
      return "bg-accent-download/15 text-accent-download";
    case "INFO":
      return "bg-success-bg/15 text-success";
    case "WARN":
      return "bg-warning-bg/20 text-warning";
    case "ERROR":
      return "bg-error-bg/15 text-error";
    default:
      return "bg-surface-sunken text-tertiary";
  }
};

const mergeBuffers = (
  a1: Uint8Array<ArrayBuffer>,
  a2: Uint8Array<ArrayBuffer>,
): Uint8Array<ArrayBuffer> => {
  if (a1.length === 0) {
    return a2;
  }
  if (a2.length === 0) {
    return a1;
  }
  const merged = new Uint8Array(a1.length + a2.length);
  merged.set(a1);
  merged.set(a2, a1.length);
  return merged;
};

const streamLogs = (
  url: string,
  addLine: (text: string) => void,
  setError: (error: ErrorWithLabel | null) => void,
): (() => void) => {
  const controller = new AbortController();
  const signal = controller.signal;

  let canceled = false;

  const cancelFetch = () => {
    console.log("cancelling fetch");
    canceled = true;
    controller.abort();
  };

  const runOnce = async () => {
    const response = await fetch(url, { signal });

    if (!response.ok) {
      const text = await response.text();
      setError({
        text: "error fetching logs",
        details: {
          statusText: response.statusText,
          text,
        },
      });
      throw null;
    }

    if (!response.body) {
      setError({
        text: "error fetching logs: ReadableStream not supported.",
      });
      return;
    }

    setError(null);

    const reader = response.body.getReader();

    let buffer = new Uint8Array();
    while (true) {
      const { done, value } = await reader.read();

      if (done) {
        setError({
          text: "log stream terminated",
        });
        throw null;
      }

      buffer = mergeBuffers(buffer, value);

      for (let newLineIdx: number; (newLineIdx = buffer.indexOf(10)) !== -1; ) {
        const lineBytes = buffer.slice(0, newLineIdx);
        const line = new TextDecoder().decode(lineBytes);
        addLine(line);
        buffer = buffer.slice(newLineIdx + 1);
      }
    }
  };

  const cancelLoop = loopUntilSuccess(
    () =>
      runOnce().then(
        () => {},
        (e) => {
          if (canceled) {
            return;
          }
          if (e === null) {
            // We already set the error.
            return;
          }
          setError({
            text: "error streaming logs",
            details: {
              text: e.toString(),
            },
          });
          throw e;
        },
      ),
    1000,
  );

  return () => {
    cancelFetch();
    cancelLoop();
  };
};

export const LogStream: React.FC<LogStreamProps> = ({
  url,
  maxLines,
  torrentFilter,
}) => {
  const [logLines, setLogLines] = useState<Line[]>([]);
  const [error, setError] = useState<ErrorWithLabel | null>(null);
  const [filter, setFilter] = useState<string>("");
  const filterRegex = useRef<RegExp | null>(null);
  const [enabledLevels, setEnabledLevels] = useState<Set<string>>(
    new Set(["TRACE", "DEBUG", "INFO", "WARN", "ERROR"]),
  );

  const maxL = maxLines ?? 1000;

  const toggleLevel = useCallback((level: string) => {
    setEnabledLevels((prev) => {
      const next = new Set(prev);
      if (next.has(level)) next.delete(level);
      else next.add(level);
      return next;
    });
  }, []);

  const addLine = useCallback(
    (text: string) => {
      setLogLines((logLines: Line[]) => {
        const nextLineId = logLines.length == 0 ? 0 : logLines[0].id + 1;
        // Read filterRegex.current inside the state updater to avoid stale closure
        const currentFilter = filterRegex.current;

        const newLogLines = [
          {
            id: nextLineId,
            content: text,
            parsed: JSON.parse(text) as JSONLogLine,
            show: currentFilter ? !!text.match(currentFilter) : true,
          },
          ...logLines.slice(0, maxL - 1),
        ];
        return newLogLines;
      });
    },
    [maxLines],
  );

  const addLineRef = useRef(addLine);
  addLineRef.current = addLine;

  const updateFilter = debounce((value: string) => {
    let regex: RegExp | null = null;
    try {
      regex = new RegExp(value);
    } catch (e) {
      return;
    }
    filterRegex.current = regex;
    setLogLines((logLines) => {
      const tmp = [...logLines];
      for (const line of tmp) {
        line.show = !!line.content.match(regex as RegExp);
      }
      return tmp;
    });
  }, 200);

  const handleFilterChange = (value: string) => {
    setFilter(value);
    updateFilter(value);
  };

  useEffect(() => updateFilter.cancel, []);

  useEffect(() => {
    return streamLogs(url, (line) => addLineRef.current(line), setError);
  }, [url]);

  const filteredLines = useMemo(
    () =>
      logLines.filter(
        (line) =>
          line.show &&
          enabledLevels.has(line.parsed.level) &&
          (!torrentFilter ||
            lineMatchesTorrent(line.parsed, line.content, torrentFilter)),
      ),
    [logLines, enabledLevels, torrentFilter],
  );

  const copyLogs = useCallback(() => {
    const text = filteredLines.map((l) => l.content).join("\n");
    navigator.clipboard.writeText(text);
  }, [filteredLines]);

  const downloadLogs = useCallback(() => {
    const text = filteredLines.map((l) => l.content).join("\n");
    const blob = new Blob([text], { type: "text/plain" });
    const blobUrl = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = blobUrl;
    a.download = `rusttorrent-logs-${new Date().toISOString().slice(0, 19)}.log`;
    a.click();
    URL.revokeObjectURL(blobUrl);
  }, [filteredLines]);

  const levelBtnInactive = "bg-surface-sunken text-tertiary";

  return (
    <div className="flex flex-col h-full">
      <ErrorComponent error={error} />

      {/* Controls bar */}
      <div className="flex items-center gap-2 mb-2 flex-wrap">
        {/* Level filter buttons */}
        {LOG_LEVELS.map((level) => (
          <button
            key={level}
            onClick={() => toggleLevel(level)}
            className={`px-2 py-0.5 text-xs font-mono rounded cursor-pointer ${
              enabledLevels.has(level)
                ? levelButtonActiveClass(level)
                : levelBtnInactive
            }`}
          >
            {level}
          </button>
        ))}

        {/* Spacer */}
        <div className="flex-1" />

        {/* Copy/Download */}
        <button
          onClick={copyLogs}
          className="text-xs text-secondary hover:text-primary cursor-pointer"
        >
          Copy
        </button>
        <button
          onClick={downloadLogs}
          className="text-xs text-secondary hover:text-primary cursor-pointer"
        >
          Download
        </button>
      </div>

      {/* Regex filter */}
      <div className="mb-2">
        <input
          value={filter}
          onChange={(e) => handleFilterChange(e.target.value)}
          placeholder="Filter (regex)..."
          className="w-full px-3 py-1.5 text-sm bg-surface border border-divider rounded focus:outline-none focus:border-primary"
        />
      </div>

      {/* Info line */}
      <div className="text-xs text-tertiary mb-1">
        Showing {filteredLines.length} of {logLines.length} lines (last {maxL}{" "}
        since window opened)
      </div>

      {/* Virtualized log output */}
      <div className="flex-1 min-h-0" style={{ minHeight: "300px" }}>
        <Virtuoso
          data={filteredLines}
          followOutput="smooth"
          itemContent={(_, line) => <LogLine line={line.parsed} />}
        />
      </div>
    </div>
  );
};
