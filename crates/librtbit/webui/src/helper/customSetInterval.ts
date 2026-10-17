// Run a function with initial interval, then run it forever with the interval that the
// callback returns.
// While the page is hidden, the delay is stretched to at least HIDDEN_MIN_INTERVAL_MS.
// Returning to the page runs the callback straight away.
// Returns a callback to clear it.

export const HIDDEN_MIN_INTERVAL_MS = 30_000;

export function customSetInterval(
  asyncCallback: () => Promise<number>,
  initialInterval: number,
): () => void {
  let timeoutId: any;
  let currentInterval: number = initialInterval;
  let running = false;
  let stopped = false;

  const executeCallback = async () => {
    if (stopped) return;
    running = true;
    try {
      currentInterval = await asyncCallback();
    } finally {
      running = false;
    }
    if (currentInterval === null || currentInterval === undefined) {
      throw "asyncCallback returned null or undefined";
    }
    scheduleNext();
  };

  const scheduleNext = () => {
    if (stopped) return;
    const delay = document.hidden
      ? Math.max(currentInterval, HIDDEN_MIN_INTERVAL_MS)
      : currentInterval;
    timeoutId = setTimeout(executeCallback, delay);
  };

  const onVisibilityChange = () => {
    if (document.hidden || stopped) return;
    // A run already in flight schedules the next one itself, so only
    // pull the pending timer forward when nothing is running.
    if (running) return;
    clearTimeout(timeoutId);
    executeCallback();
  };

  scheduleNext();
  document.addEventListener("visibilitychange", onVisibilityChange);

  return () => {
    stopped = true;
    clearTimeout(timeoutId);
    document.removeEventListener("visibilitychange", onVisibilityChange);
  };
}
