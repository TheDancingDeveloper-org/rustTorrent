// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";

import { SHORTCUTS } from "./shortcutList.ts";

test("every shortcut has a key label and an action", () => {
  for (const entry of SHORTCUTS) {
    assert.ok(entry.keys.trim().length > 0);
    assert.ok(entry.action.trim().length > 0);
  }
});

test("shortcut key labels are unique", () => {
  const keys = SHORTCUTS.map((entry) => entry.keys);
  assert.equal(new Set(keys).size, keys.length);
});
