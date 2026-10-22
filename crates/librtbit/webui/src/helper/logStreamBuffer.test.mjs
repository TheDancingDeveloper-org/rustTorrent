// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import {
  DEFAULT_ENABLED_LEVELS,
  LOG_BUFFER_CAP,
  linesSincePause,
  prependBounded,
  visibleLines,
} from "./logStreamBuffer.ts";

// Newest-first lines with ids `from` down to `to`.
const lines = (from, to) => {
  const out = [];
  for (let id = from; id >= to; id--) {
    out.push({ id, content: `line ${id}` });
  }
  return out;
};

test("log view opens on INFO and above, with DEBUG and TRACE off", () => {
  assert.deepEqual([...DEFAULT_ENABLED_LEVELS], ["INFO", "WARN", "ERROR"]);
});

test("prependBounded keeps newest first and drops the oldest past the cap", () => {
  const next = prependBounded(lines(2, 0), lines(4, 3), 4);
  assert.deepEqual(
    next.map((l) => l.id),
    [4, 3, 2, 1],
  );
});

test("prependBounded returns the same array when nothing arrives", () => {
  const current = lines(2, 0);
  assert.equal(prependBounded(current, [], 10), current);
});

test("the default cap is a few thousand lines", () => {
  assert.ok(LOG_BUFFER_CAP >= 1000 && LOG_BUFFER_CAP <= 5000);
  const buffer = prependBounded(
    [],
    lines(LOG_BUFFER_CAP + 499, 0),
    LOG_BUFFER_CAP,
  );
  assert.equal(buffer.length, LOG_BUFFER_CAP);
  assert.equal(buffer[0].id, LOG_BUFFER_CAP + 499);
});

test("not paused: every buffered line is visible and none are counted new", () => {
  const buffer = lines(5, 0);
  assert.equal(visibleLines(buffer, null), buffer);
  assert.equal(linesSincePause(buffer, null), 0);
});

test("paused: lines that arrive after the pause stay hidden and are counted", () => {
  // Paused with newest id 5; ids 6, 7 and 8 then stream in.
  const buffer = prependBounded(lines(5, 0), lines(8, 6), LOG_BUFFER_CAP);
  assert.deepEqual(
    visibleLines(buffer, 5).map((l) => l.id),
    [5, 4, 3, 2, 1, 0],
  );
  assert.equal(linesSincePause(buffer, 5), 3);
});

test("resume shows the latest lines again and clears the count", () => {
  const buffer = prependBounded(lines(5, 0), lines(8, 6), LOG_BUFFER_CAP);
  assert.equal(visibleLines(buffer, null).length, buffer.length);
  assert.equal(visibleLines(buffer, null)[0].id, 8);
  assert.equal(linesSincePause(buffer, null), 0);
});

test("paused count is bounded by the buffer cap", () => {
  const paused = lines(10, 0);
  let buffer = paused;
  // 2 batches of 4 new lines through a cap of 12: only the newest 12 remain.
  buffer = prependBounded(buffer, lines(14, 11), 12);
  buffer = prependBounded(buffer, lines(18, 15), 12);
  assert.equal(buffer.length, 12);
  assert.equal(linesSincePause(buffer, 10), 8);
});
