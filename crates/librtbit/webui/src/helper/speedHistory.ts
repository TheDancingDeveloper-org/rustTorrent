// A fixed-length series of session download and upload speeds (Mbps) for the
// live graph. Samples arrive in poll order; the oldest falls off the front.

export interface SpeedSample {
  downMbps: number;
  upMbps: number;
}

export function appendSpeedSample(
  history: readonly SpeedSample[],
  sample: SpeedSample,
  capacity: number,
): SpeedSample[] {
  const next = history.length >= capacity ? history.slice(1) : history.slice();
  next.push({
    downMbps: finite(sample.downMbps),
    upMbps: finite(sample.upMbps),
  });
  return next;
}

function finite(value: number): number {
  return Number.isFinite(value) && value > 0 ? value : 0;
}

/** Peak across both series, at least 1 Mbps so an idle graph still has a scale. */
export function speedScale(history: readonly SpeedSample[]): number {
  let peak = 1;
  for (const sample of history) {
    peak = Math.max(peak, sample.downMbps, sample.upMbps);
  }
  return peak;
}

export interface SpeedPoint {
  x: number;
  y: number;
}

/** Map one series onto a width×height box. Newest sample is at the right edge. */
export function speedPolyline(
  history: readonly SpeedSample[],
  series: "downMbps" | "upMbps",
  width: number,
  height: number,
  scale: number,
): SpeedPoint[] {
  if (history.length === 0 || width <= 0 || height <= 0 || scale <= 0) {
    return [];
  }
  const last = Math.max(history.length - 1, 1);
  return history.map((sample, index) => ({
    x: (index / last) * width,
    y: height - (sample[series] / scale) * height,
  }));
}

export function polylineAttr(points: readonly SpeedPoint[]): string {
  return points.map((point) => `${point.x},${point.y}`).join(" ");
}
