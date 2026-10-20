// @ts-nocheck
// Guards the settings, modal and auth theming (UI fix WP-G): theme tokens
// instead of raw palette colours, visible focus rings, body colour for plain
// data, and the dead AdvancedTab staying deleted.
import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";

const path = (p) => fileURLToPath(new URL(p, import.meta.url));
const read = (p) => readFileSync(path(p), "utf8");

const configFiles = readdirSync(path("../components/config"))
  .filter((f) => f.endsWith(".tsx"))
  .map((f) => `../components/config/${f}`);

const FILES = [
  "../components/Tabs.tsx",
  "../components/modal/Modal.tsx",
  "../components/modal/TabbedConfigModal.tsx",
  "../components/auth/LoginPage.tsx",
  "../components/auth/SetupPage.tsx",
  ...configFiles,
];

const RAW_PALETTE =
  /\b(?:[a-z]+:)*(?:text|bg|border|ring|accent|divide)-(?:slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)-\d{2,3}\b/g;

test("settings, modal and auth files use theme tokens", () => {
  for (const file of FILES) {
    const hits = read(file).match(RAW_PALETTE) ?? [];
    assert.deepEqual(hits, [], `${file} has raw palette classes`);
  }
});

test("form fields keep a visible keyboard focus", () => {
  for (const file of FILES) {
    const src = read(file);
    assert.doesNotMatch(src, /focus:outline-none/, `${file}: focus:outline-none`);
    assert.doesNotMatch(src, /focus:ring-0/, `${file}: focus:ring-0`);
  }
});

test("plain data in the info tabs renders in body colour", () => {
  for (const file of ["GeneralTab", "DHTTab", "ConnectionTab"]) {
    const src = read(`../components/config/${file}.tsx`);
    assert.doesNotMatch(src, /\btext-primary\b/, `${file} uses text-primary`);
  }
});

test("RSS settings headings sit under the tab heading level", () => {
  assert.doesNotMatch(read("../components/config/RSSTab.tsx"), /<h4\b/);
});

test("the unused AdvancedTab stays deleted", () => {
  assert.equal(existsSync(path("../components/config/AdvancedTab.tsx")), false);
});
