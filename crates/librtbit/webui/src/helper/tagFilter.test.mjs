// @ts-nocheck
import { test } from "node:test";
import assert from "node:assert/strict";
// matchesTag is duplicated here rather than imported from torrentFilters.ts,
// which pulls in statusFilter.ts — node:test cannot resolve that extensionless
// import (same reason statusFilters.test.mjs imports ./statusFilter.ts directly).
function matchesTag(torrent, tagFilter) {
  if (tagFilter === null) return true;
  const tags = torrent.tags ?? [];
  if (tagFilter === "") return tags.length === 0;
  return tags.includes(tagFilter);
}


const torrent = (tags) => ({ id: 1, name: "x", tags });

test("null filter shows every torrent", () => {
  assert.ok(matchesTag(torrent(["hd"]), null));
  assert.ok(matchesTag(torrent([]), null));
  assert.ok(matchesTag(torrent(undefined), null));
});

test("empty filter shows only untagged torrents", () => {
  assert.ok(matchesTag(torrent([]), ""));
  assert.ok(matchesTag(torrent(undefined), ""));
  assert.ok(!matchesTag(torrent(["hd"]), ""));
});

test("a named filter matches torrents carrying that tag", () => {
  assert.ok(matchesTag(torrent(["hd", "tv"]), "tv"));
  assert.ok(!matchesTag(torrent(["hd"]), "tv"));
  assert.ok(!matchesTag(torrent(undefined), "tv"));
});
