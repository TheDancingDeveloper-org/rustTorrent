import { useEffect, useState } from "react";
import { Speed } from "../api-types";
import {
  appendSpeedSample,
  polylineAttr,
  speedPolyline,
  speedScale,
  SpeedSample,
} from "../helper/speedHistory";

const CAPACITY = 60;
const WIDTH = 640;
const HEIGHT = 160;

export const SpeedGraph: React.FC<{
  download: Speed;
  upload: Speed;
}> = ({ download, upload }) => {
  const [history, setHistory] = useState<SpeedSample[]>([]);

  useEffect(() => {
    setHistory((prev) =>
      appendSpeedSample(
        prev,
        { downMbps: download.mbps, upMbps: upload.mbps },
        CAPACITY,
      ),
    );
  }, [download.mbps, upload.mbps, download.human_readable, upload.human_readable]);

  const scale = speedScale(history);
  const down = polylineAttr(
    speedPolyline(history, "downMbps", WIDTH, HEIGHT, scale),
  );
  const up = polylineAttr(
    speedPolyline(history, "upMbps", WIDTH, HEIGHT, scale),
  );

  return (
    <section aria-label="Live speed">
      <div className="mb-2 flex flex-wrap items-baseline gap-x-4 gap-y-1 text-sm">
        <h3 className="font-medium text-secondary">Live speed</h3>
        <span className="text-accent-download">
          Down {download.human_readable}
        </span>
        <span className="text-accent-upload">Up {upload.human_readable}</span>
        <span className="text-xs text-tertiary">
          last {CAPACITY} samples, scale {scale.toFixed(1)} Mbps
        </span>
      </div>
      <svg
        viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
        className="h-40 w-full rounded-lg border border-divider bg-surface-raised"
        role="img"
        aria-label={`Download ${download.human_readable}, upload ${upload.human_readable}`}
      >
        <polyline
          points={down}
          fill="none"
          className="stroke-accent-download"
          strokeWidth="2"
        />
        <polyline
          points={up}
          fill="none"
          className="stroke-accent-upload"
          strokeWidth="2"
        />
      </svg>
    </section>
  );
};
