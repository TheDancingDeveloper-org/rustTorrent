// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";

import {
  SETTINGS,
  restartRequiredSettings,
  settingLabel,
} from "../components/config/settingsRegistry.ts";

test("setting ids are unique", () => {
  const ids = SETTINGS.map((s) => s.id);
  assert.equal(new Set(ids).size, ids.length);
});

test("both bind addresses are restart-required", () => {
  const bind = SETTINGS.filter((s) => /bind address/i.test(s.label));
  assert.equal(bind.length, 2);
  for (const setting of bind) {
    assert.equal(setting.restartRequired, true, setting.label);
  }
});

test("settingLabel reads the registry label", () => {
  assert.equal(settingLabel("peer-limit"), "Peer limit");
});

test("settingLabel throws on an unknown id", () => {
  assert.throws(() => settingLabel("no-such-setting"), /Unknown setting id/);
});

test("restartRequiredSettings filters by tab", () => {
  const tabs = new Set(
    restartRequiredSettings("connections").map((s) => s.tab),
  );
  assert.deepEqual([...tabs], ["connections"]);
});
