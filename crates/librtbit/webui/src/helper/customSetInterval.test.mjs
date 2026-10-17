// @ts-nocheck
import test, { mock } from "node:test";
import assert from "node:assert/strict";
import {
  customSetInterval,
  HIDDEN_MIN_INTERVAL_MS,
} from "./customSetInterval.ts";

// Minimal stand-in for the browser document: visibility state plus events.
// Installed once: teardown order of t.after hooks can run clear() after a
// per-test deletion, and clear() still touches the document.
const doc = new EventTarget();
doc.hidden = false;
globalThis.document = doc;

// Let promise continuations (the awaited callback result) run.
const flush = () => new Promise((resolve) => setImmediate(resolve));

const setup = (t) => {
  doc.hidden = false;
  mock.timers.enable({ apis: ["setTimeout"] });
  t.after(() => mock.timers.reset());
  return doc;
};

const setVisibility = (doc, hidden) => {
  doc.hidden = hidden;
  doc.dispatchEvent(new Event("visibilitychange"));
};

test("runs at the returned interval while visible", async (t) => {
  setup(t);
  let calls = 0;
  const clear = customSetInterval(async () => {
    calls++;
    return 1000;
  }, 0);
  t.after(clear);

  mock.timers.tick(1);
  await flush();
  assert.equal(calls, 1);

  mock.timers.tick(999);
  await flush();
  assert.equal(calls, 1);

  mock.timers.tick(1);
  await flush();
  assert.equal(calls, 2);
});

test("stretches the delay to at least 30 s while hidden", async (t) => {
  const doc = setup(t);
  doc.hidden = true;
  let calls = 0;
  const clear = customSetInterval(async () => {
    calls++;
    return 1000;
  }, 0);
  t.after(clear);

  mock.timers.tick(HIDDEN_MIN_INTERVAL_MS);
  await flush();
  assert.equal(calls, 1);

  mock.timers.tick(HIDDEN_MIN_INTERVAL_MS - 1);
  await flush();
  assert.equal(calls, 1);

  mock.timers.tick(1);
  await flush();
  assert.equal(calls, 2);
});

test("keeps a longer interval while hidden", async (t) => {
  const doc = setup(t);
  doc.hidden = true;
  let calls = 0;
  const clear = customSetInterval(async () => {
    calls++;
    return 60_000;
  }, 0);
  t.after(clear);

  mock.timers.tick(HIDDEN_MIN_INTERVAL_MS);
  await flush();
  assert.equal(calls, 1);

  mock.timers.tick(59_999);
  await flush();
  assert.equal(calls, 1);

  mock.timers.tick(1);
  await flush();
  assert.equal(calls, 2);
});

test("returning to the page runs immediately and drops the pending timer", async (t) => {
  const doc = setup(t);
  doc.hidden = true;
  let calls = 0;
  const clear = customSetInterval(async () => {
    calls++;
    return 1000;
  }, 0);
  t.after(clear);

  mock.timers.tick(HIDDEN_MIN_INTERVAL_MS);
  await flush();
  assert.equal(calls, 1);

  setVisibility(doc, false);
  await flush();
  assert.equal(calls, 2, "visible again should run straight away");

  // The old 30 s timer must not fire a second chain on top of the new one.
  mock.timers.tick(1000);
  await flush();
  assert.equal(calls, 3);
});

test("clearing stops further runs and removes the listener", async (t) => {
  const doc = setup(t);
  let calls = 0;
  const clear = customSetInterval(async () => {
    calls++;
    return 1000;
  }, 0);

  mock.timers.tick(1);
  await flush();
  assert.equal(calls, 1);

  clear();
  setVisibility(doc, false);
  mock.timers.tick(60_000);
  await flush();
  assert.equal(calls, 1);
});
