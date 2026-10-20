import { useContext, useEffect, useState } from "react";
import { FaArrowDown, FaArrowUp } from "react-icons/fa";
import { BsPeople, BsClock, BsSpeedometer2 } from "react-icons/bs";
import { APIContext } from "../context";
import { formatBytes } from "../helper/formatBytes";
import { formatSecondsToTime } from "../helper/formatSecondsToTime";
import { useStatsStore } from "../stores/statsStore";

/** Turtle icon for alternative speed limits (qBittorrent-style). */
const TurtleIcon: React.FC<{ className?: string }> = ({ className }) => (
  <svg viewBox="0 0 24 24" fill="currentColor" className={className}>
    <path d="M8.5 7.5C9.6 6.1 11.5 5 13.5 5c3.6 0 6.5 2.9 6.5 6.5 0 .5-.1 1-.2 1.5H21c.6 0 1 .4 1 1s-.4 1-1 1h-1.9c-.8 1.2-2 2.1-3.4 2.6l.6 1.2c.2.5 0 1.1-.5 1.3-.5.2-1.1 0-1.3-.5l-.7-1.4c-.1 0-.2 0-.3 0h-3l-.7 1.4c-.2.5-.8.7-1.3.5-.5-.2-.7-.8-.5-1.3l.5-1.1c-.9-.4-1.7-.9-2.3-1.6l-1.4.9c-.5.3-1.1.2-1.4-.3-.3-.5-.2-1.1.3-1.4l1.5-1c-.1-.4-.2-.9-.2-1.3H3c-.6 0-1-.4-1-1s.4-1 1-1h2.1c.2-.9.6-1.7 1.1-2.4l-.9-.9c-.4-.4-.4-1 0-1.4.4-.4 1-.4 1.4 0l.9.9c.3-.2.6-.4.9-.6zm5 .5c-2.5 0-4.5 2-4.5 4.5S11 17 13.5 17s4.5-2 4.5-4.5S16 8 13.5 8z" />
  </svg>
);

const FooterPiece: React.FC<{
  children: React.ReactNode;
  title?: string;
}> = ({ children, title }) => {
  return (
    <div
      className="flex items-center gap-1.5 px-2 py-1 whitespace-nowrap"
      title={title}
    >
      {children}
    </div>
  );
};

export const Footer: React.FC = () => {
  const API = useContext(APIContext);
  const stats = useStatsStore((stats) => stats.stats);

  // Alternative speed limits toggle
  const [altSupported, setAltSupported] = useState(false);
  const [altEnabled, setAltEnabled] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const poll = () =>
      API.getAltSpeed()
        .then((status) => {
          if (cancelled) return;
          setAltSupported(true);
          setAltEnabled(status.enabled);
        })
        .catch(() => {
          if (!cancelled) setAltSupported(false);
        });
    poll();
    const interval = setInterval(poll, 10000);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [API]);

  const toggleAlt = async () => {
    const next = !altEnabled;
    setAltEnabled(next);
    try {
      await API.toggleAltSpeed(next);
    } catch {
      setAltEnabled(!next);
    }
  };

  return (
    <div className="bg-surface-raised border-t border-divider text-sm text-secondary flex items-center flex-wrap gap-x-1 lg:gap-x-3 px-1">
      {altSupported && (
        <button
          type="button"
          onClick={toggleAlt}
          aria-label="Alternative speed limits"
          aria-pressed={altEnabled}
          className={`p-1 rounded cursor-pointer transition-colors focus-ring ${
            altEnabled
              ? "text-primary bg-primary/15"
              : "text-tertiary hover:text-secondary hover:bg-surface-sunken"
          }`}
          title={
            altEnabled
              ? "Alternative speed limits are ON — click to disable"
              : "Alternative speed limits are OFF — click to enable"
          }
        >
          <TurtleIcon className="w-4 h-4" />
        </button>
      )}

      <div className="flex-1 flex items-center justify-evenly flex-wrap gap-x-1 lg:gap-x-5">
        <FooterPiece title="Download speed (session total)">
          <FaArrowDown className="w-3 h-3 text-accent-download" />
          <span className="text-accent-download font-medium tabular-nums">
            {stats.download_speed.human_readable}
          </span>
          <span className="text-tertiary">
            ({formatBytes(stats.counters.fetched_bytes)})
          </span>
        </FooterPiece>
        <FooterPiece title="Upload speed (session total)">
          <FaArrowUp className="w-3 h-3 text-accent-upload" />
          <span className="text-accent-upload font-medium tabular-nums">
            {stats.upload_speed.human_readable}
          </span>
          <span className="text-tertiary">
            ({formatBytes(stats.counters.uploaded_bytes)})
          </span>
        </FooterPiece>
        <FooterPiece title="Connected peers">
          <BsPeople className="w-3.5 h-3.5 text-tertiary" />
          <span className="tabular-nums">{stats.peers.live}</span>
        </FooterPiece>
        <FooterPiece title="Session uptime">
          <BsClock className="w-3.5 h-3.5 text-tertiary" />
          <span className="tabular-nums">
            {formatSecondsToTime(stats.uptime_seconds)}
          </span>
        </FooterPiece>
        <FooterPiece>
          <BsSpeedometer2 className="w-3.5 h-3.5 text-tertiary" />
          <a
            href="/swagger/"
            target="_blank"
            className="text-primary hover:underline"
          >
            API
          </a>
        </FooterPiece>
      </div>
    </div>
  );
};
