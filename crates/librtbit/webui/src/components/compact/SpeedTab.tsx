import React, { useEffect, useRef, useState, useCallback } from "react";
import { TorrentListItem } from "../../api-types";
import { formatBytes } from "../../helper/formatBytes";

interface SpeedTabProps {
  torrent: TorrentListItem | null;
}

interface SpeedPoint {
  timestamp: number;
  downloadSpeed: number; // bytes per second
  uploadSpeed: number;
}

const MAX_POINTS = 300; // 5 minutes at 1 point/second

export const SpeedTab: React.FC<SpeedTabProps> = ({ torrent }) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [speedHistory, setSpeedHistory] = useState<SpeedPoint[]>([]);
  // Bumped when the theme class on <body> flips, so the chart redraws in the new colours.
  const [themeVersion, setThemeVersion] = useState(0);

  // Reset history when torrent changes
  useEffect(() => {
    setSpeedHistory([]);
  }, [torrent?.id]);

  // Build history from the torrent list refresh rather than a separate poll.
  // The store keeps the same torrent reference when a refresh is unchanged, so
  // each new reference is a fresh sample.
  useEffect(() => {
    const live = torrent?.stats?.live;
    if (!live || torrent?.stats?.state !== "live") return;

    setSpeedHistory((prev) => {
      const next = [
        ...prev,
        {
          timestamp: Date.now(),
          downloadSpeed: (live.download_speed.mbps * 1024 * 1024) / 8,
          uploadSpeed: (live.upload_speed.mbps * 1024 * 1024) / 8,
        },
      ];
      return next.slice(-MAX_POINTS);
    });
  }, [torrent]);

  // Redraw when the theme changes
  useEffect(() => {
    const observer = new MutationObserver(() =>
      setThemeVersion((v) => v + 1),
    );
    observer.observe(document.body, {
      attributes: true,
      attributeFilter: ["class"],
    });
    return () => observer.disconnect();
  }, []);

  // Draw chart
  const drawChart = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || speedHistory.length < 2) return;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const width = canvas.clientWidth;
    const height = canvas.clientHeight;
    canvas.width = width * dpr;
    canvas.height = height * dpr;
    ctx.scale(dpr, dpr);

    // Clear
    ctx.clearRect(0, 0, width, height);

    // Find max speed for y-axis scaling
    const maxSpeed = Math.max(
      ...speedHistory.map((p) => Math.max(p.downloadSpeed, p.uploadSpeed)),
      1024, // Minimum 1 KB/s scale
    );

    // Theme colours come from the accent tokens, read from body where .dark is applied
    const computedStyle = getComputedStyle(document.body);
    const downloadColor = computedStyle
      .getPropertyValue("--color-accent-download")
      .trim();
    const uploadColor = computedStyle
      .getPropertyValue("--color-accent-upload")
      .trim();

    // Draw grid lines
    const isDark = document.body.classList.contains("dark");
    ctx.strokeStyle = isDark ? "rgba(255,255,255,0.1)" : "rgba(0,0,0,0.1)";
    ctx.lineWidth = 1;
    for (let i = 0; i < 5; i++) {
      const y = (i / 4) * height;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(width, y);
      ctx.stroke();
    }

    // Draw speed lines
    const drawLine = (
      points: SpeedPoint[],
      getValue: (p: SpeedPoint) => number,
      color: string,
    ) => {
      ctx.beginPath();
      ctx.strokeStyle = color;
      ctx.lineWidth = 2;
      points.forEach((point, i) => {
        const x = (i / (MAX_POINTS - 1)) * width;
        const y = height - (getValue(point) / maxSpeed) * height;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
      ctx.stroke();
    };

    drawLine(speedHistory, (p) => p.downloadSpeed, downloadColor);
    drawLine(speedHistory, (p) => p.uploadSpeed, uploadColor);

    // Y-axis labels
    ctx.fillStyle = isDark ? "rgba(255,255,255,0.5)" : "rgba(0,0,0,0.5)";
    ctx.font = "10px monospace";
    ctx.textAlign = "right";
    for (let i = 0; i <= 4; i++) {
      const value = maxSpeed * (1 - i / 4);
      const y = (i / 4) * height + 10;
      ctx.fillText(`${formatBytes(value)}/s`, width - 4, y);
    }
  }, [speedHistory]);

  useEffect(() => {
    drawChart();
  }, [drawChart, themeVersion]);

  if (!torrent) return null;

  if (torrent.stats?.state !== "live") {
    return (
      <div className="p-3 text-tertiary text-sm">
        Speed chart is only available for active torrents.
      </div>
    );
  }

  return (
    <div className="p-3 h-full flex flex-col">
      <div className="flex items-center gap-4 mb-2 text-xs">
        <div className="flex items-center gap-1">
          <span className="w-3 h-0.5 bg-accent-download inline-block rounded" />
          <span className="text-secondary">Download</span>
          {torrent.stats?.live && (
            <span className="text-primary font-medium">
              {torrent.stats.live.download_speed.human_readable}
            </span>
          )}
        </div>
        <div className="flex items-center gap-1">
          <span className="w-3 h-0.5 bg-accent-upload inline-block rounded" />
          <span className="text-secondary">Upload</span>
          {torrent.stats?.live && (
            <span className="text-primary font-medium">
              {torrent.stats.live.upload_speed.human_readable}
            </span>
          )}
        </div>
      </div>
      <div className="flex-1 min-h-0">
        <canvas
          ref={canvasRef}
          className="w-full h-full bg-surface-sunken rounded"
          style={{ minHeight: "120px" }}
        />
      </div>
    </div>
  );
};
