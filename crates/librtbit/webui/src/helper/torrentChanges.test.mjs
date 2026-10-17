// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";

import { describeTorrentChanges } from "./torrentChanges.ts";

const snap = (name, state = "live", error = null) => ({ name, state, error });
const map = (entries) => new Map(entries);

test("reports an added torrent", () => {
  const prev = map([[1, snap("a")]]);
  const next = map([
    [1, snap("a")],
    [2, snap("b")],
  ]);
  assert.deepEqual(describeTorrentChanges(prev, next), ["Added b"]);
});

test("reports pause and resume transitions only", () => {
  const prev = map([
    [1, snap("a", "live")],
    [2, snap("b", "paused")],
  ]);
  const next = map([
    [1, snap("a", "paused")],
    [2, snap("b", "live")],
  ]);
  assert.deepEqual(describeTorrentChanges(prev, next), [
    "Paused a",
    "Resumed b",
  ]);
});

test("reports a removed torrent", () => {
  const prev = map([[1, snap("a")]]);
  assert.deepEqual(describeTorrentChanges(prev, map([])), ["Removed a"]);
});

test("reports an error with its message", () => {
  const prev = map([[1, snap("a", "live")]]);
  const next = map([[1, snap("a", "error", "disk full")]]);
  assert.deepEqual(describeTorrentChanges(prev, next), [
    "Error on a: disk full",
  ]);
});

test("stays quiet when nothing changed", () => {
  const same = map([[1, snap("a")]]);
  assert.deepEqual(describeTorrentChanges(same, same), []);
});

test("collapses a burst of changes into one count", () => {
  const prev = map([]);
  const next = map([1, 2, 3, 4].map((id) => [id, snap(`t${id}`)]));
  assert.deepEqual(describeTorrentChanges(prev, next), ["4 torrent changes"]);
});
