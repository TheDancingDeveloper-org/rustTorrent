// @ts-nocheck
// Guards the feature-page theming (UI fix WP-F): the RSS, Indexarr, log and
// delete/upload modal components use theme tokens rather than raw palette
// colours, and the categorical Indexarr badges, the one justified exception,
// keep WCAG AA (4.5:1) contrast in both themes.
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const read = (path) =>
  readFileSync(fileURLToPath(new URL(path, import.meta.url)), "utf8");

const FILES = [
  "../components/RSSPage.tsx",
  "../components/IndexarrBrowse.tsx",
  "../components/IndexarrSetup.tsx",
  "../components/LogStream.tsx",
  "../components/modal/DeleteTorrentModal.tsx",
  "../components/modal/MultiTorrentUploadModal.tsx",
];

const RAW_PALETTE =
  /\b(?:[a-z]+:)*(?:text|bg|border|ring|from|to|via|fill|stroke|accent|divide)-(?:slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)-\d{2,3}\b/g;

// Source of the categorical badge map, the only place raw steps are allowed.
function badgeBlock(src) {
  const match = src.match(
    /const colors: Record<string, string> = \{([\s\S]*?)\};/,
  );
  assert.ok(match, "contentTypeBadge colour map not found");
  return match[1];
}

test("feature pages use theme tokens, not raw palette colours", () => {
  for (const file of FILES) {
    let src = read(file);
    if (file.endsWith("IndexarrBrowse.tsx")) {
      src = src.replace(badgeBlock(src), "");
    }
    const hits = src.match(RAW_PALETTE) ?? [];
    assert.deepEqual(hits, [], `${file} has raw palette classes`);
  }
});

test("RSS headings and form fields follow the shared conventions", () => {
  const src = read("../components/RSSPage.tsx");
  assert.doesNotMatch(src, /<h3\b/, "RSS modal titles should be h2");
  assert.equal(
    (src.match(/const inputCls =/g) ?? []).length,
    1,
    "one shared inputCls",
  );
  assert.match(src, /const fieldCls =\s*"[^"]*\bfocus-ring\b/);
  assert.doesNotMatch(src, /focus:outline-none/);
});

// Tailwind palette values (hex approximations of the v4 oklch steps).
const PALETTE = {
  "blue-300": "#93c5fd",
  "blue-500": "#3b82f6",
  "blue-800": "#1e40af",
  "purple-300": "#d8b4fe",
  "purple-500": "#a855f7",
  "purple-800": "#6b21a8",
  "green-300": "#86efac",
  "green-500": "#22c55e",
  "green-800": "#166534",
  "yellow-300": "#fde047",
  "yellow-500": "#eab308",
  "amber-800": "#92400e",
  "red-300": "#fca5a5",
  "red-500": "#ef4444",
  "red-800": "#991b1b",
  "cyan-300": "#67e8f9",
  "cyan-500": "#06b6d4",
  "cyan-800": "#155e75",
  "pink-300": "#f9a8d4",
  "pink-500": "#ec4899",
  "pink-800": "#9d174d",
};

// Surfaces a result row can sit on: surface and surface-raised per theme.
const LIGHT_SURFACES = ["#ffffff", "#f8fafc"];
const DARK_SURFACES = ["#0f172a", "#1e293b"];

const rgb = (hex) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);

function luminance(channels) {
  const [r, g, b] = channels.map((c) =>
    c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4,
  );
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

const blend = (fg, bg, alpha) =>
  rgb(fg).map((c, i) => alpha * c + (1 - alpha) * rgb(bg)[i]);

function contrast(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

test("Indexarr category badges reach 4.5:1 in both themes", () => {
  const block = badgeBlock(read("../components/IndexarrBrowse.tsx"));
  const rows = [...block.matchAll(/(\w+): "([^"]+)"/g)];
  assert.ok(rows.length >= 7, "expected every content type in the map");

  for (const [, type, classes] of rows) {
    const bg = classes.match(/\bbg-([a-z]+-\d+)\/(\d+)\b/);
    const light = classes.match(/(?:^|\s)text-([a-z]+-\d+)\b/);
    const dark = classes.match(/\bdark:text-([a-z]+-\d+)\b/);
    assert.ok(bg && light && dark, `${type}: needs bg/N, text and dark:text`);

    const tint = PALETTE[bg[1]];
    const alpha = Number(bg[2]) / 100;
    for (const [fg, surfaces] of [
      [light[1], LIGHT_SURFACES],
      [dark[1], DARK_SURFACES],
    ]) {
      assert.ok(PALETTE[fg] && tint, `${type}: no palette value for ${fg}`);
      for (const surface of surfaces) {
        const ratio = contrast(rgb(PALETTE[fg]), blend(tint, surface, alpha));
        assert.ok(
          ratio >= 4.5,
          `${type}: ${fg} on ${bg[0]} over ${surface} is ${ratio.toFixed(2)}:1`,
        );
      }
    }
  }
});
