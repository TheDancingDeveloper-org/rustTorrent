// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";

import { matchSettings } from "./settingsSearch.ts";
import {
  SETTINGS,
  restartRequiredSettings,
} from "../components/config/settingsRegistry.ts";

const tabs = [
  { id: "dht", label: "DHT" },
  { id: "speed", label: "Speed" },
  { id: "connections", label: "Connections" },
];

test("finds an individual setting by its label, case-insensitively", () => {
  const found = matchSettings(SETTINGS, tabs, "ALT DOWNLOAD");
  assert.deepEqual(
    found.map((s) => s.id),
    ["alt-download-limit"],
  );
});

test("a tab label match returns that tab's settings", () => {
  const found = matchSettings(SETTINGS, tabs, "dht");
  assert.ok(found.length >= 1);
  assert.ok(found.every((s) => s.tab === "dht"));
});

test("an empty query matches nothing", () => {
  assert.deepEqual(matchSettings(SETTINGS, tabs, "   "), []);
});

test("restart markers are exactly the listen port, bind addresses, DHT, data directory and proxy", () => {
  assert.deepEqual(
    restartRequiredSettings()
      .map((s) => s.id)
      .sort(),
    [
      "data-directory",
      "dht-enabled",
      "encryption-mode",
      "http-api-bind-address",
      "listen-ip",
      "listen-port",
      "lsd-enabled",
      "pex-enabled",
      "proxy-url",
      "upnp-port-forward",
    ].sort(),
  );
});

test("setting ids are unique", () => {
  const ids = SETTINGS.map((s) => s.id);
  assert.equal(new Set(ids).size, ids.length);
});
