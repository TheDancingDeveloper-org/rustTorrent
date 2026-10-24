// @ts-nocheck
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  matchesStatus,
  STATUS_FILTERS,
  STATUS_FILTER_LABELS,
} from "./statusFilter.ts";

const t = (state, finished, down = 0, up = 0) => ({
  id: 1,
  name: "x",
  stats: {
    state,
    finished,
    live:
      state === "live"
        ? { download_speed: { mbps: down }, upload_speed: { mbps: up } }
        : null,
  },
});

test("thirteen filters, each labelled", () => {
  assert.equal(STATUS_FILTERS.length, 13);
  for (const f of STATUS_FILTERS) assert.ok(STATUS_FILTER_LABELS[f]);
});

test("derived filters", () => {
  assert.ok(matchesStatus(t("live", false, 1), "active"));
  assert.ok(!matchesStatus(t("live", false), "active"));
  assert.ok(matchesStatus(t("live", false), "stalled_down"));
  assert.ok(matchesStatus(t("live", true), "stalled_up"));
  assert.ok(!matchesStatus(t("live", true, 0, 2), "stalled_up"));
  assert.ok(matchesStatus(t("paused", true), "completed"));
  assert.ok(matchesStatus(t("paused", false), "stopped"));
  assert.ok(matchesStatus(t("error", false), "stopped"));
  assert.ok(matchesStatus(t("live", false), "running"));
  assert.ok(matchesStatus(t("initializing", false), "checking"));
});
