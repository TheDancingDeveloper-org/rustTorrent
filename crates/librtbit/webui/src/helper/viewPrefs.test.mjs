// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";

import {
  loadViewPref,
  saveViewPref,
  oneOf,
  isBoolean,
  isStringOrNull,
} from "./viewPrefs.ts";

function installStorage(entries = {}) {
  const map = new Map(Object.entries(entries));
  globalThis.localStorage = {
    getItem: (k) => (map.has(k) ? map.get(k) : null),
    setItem: (k, v) => map.set(k, String(v)),
    removeItem: (k) => map.delete(k),
  };
  return map;
}

const isStatus = oneOf(["all", "seeding"]);

test("loadViewPref returns the fallback when nothing is stored", () => {
  installStorage();
  assert.equal(loadViewPref("statusFilter", "all", isStatus), "all");
});

test("loadViewPref returns a stored value that passes validation", () => {
  installStorage({ "rtbit.view.statusFilter": '"seeding"' });
  assert.equal(loadViewPref("statusFilter", "all", isStatus), "seeding");
});

test("loadViewPref rejects a stored value that fails validation", () => {
  installStorage({ "rtbit.view.statusFilter": '"bogus"' });
  assert.equal(loadViewPref("statusFilter", "all", isStatus), "all");
});

test("loadViewPref falls back on corrupt JSON", () => {
  installStorage({ "rtbit.view.sidebarCollapsed": "{not json" });
  assert.equal(loadViewPref("sidebarCollapsed", false, isBoolean), false);
});

test("loadViewPref falls back when storage throws", () => {
  globalThis.localStorage = {
    getItem: () => {
      throw new Error("blocked");
    },
  };
  assert.equal(loadViewPref("sidebarCollapsed", true, isBoolean), true);
});

test("saveViewPref writes JSON under the namespaced key", () => {
  const map = installStorage();
  saveViewPref("categoryFilter", "tv");
  assert.equal(map.get("rtbit.view.categoryFilter"), '"tv"');
  saveViewPref("categoryFilter", null);
  assert.equal(map.get("rtbit.view.categoryFilter"), "null");
});

test("saveViewPref swallows storage errors", () => {
  globalThis.localStorage = {
    setItem: () => {
      throw new Error("quota");
    },
  };
  assert.doesNotThrow(() => saveViewPref("sidebarCollapsed", true));
});

test("isStringOrNull accepts null and strings only", () => {
  assert.equal(isStringOrNull(null), true);
  assert.equal(isStringOrNull("tracker.example"), true);
  assert.equal(isStringOrNull(3), false);
});
