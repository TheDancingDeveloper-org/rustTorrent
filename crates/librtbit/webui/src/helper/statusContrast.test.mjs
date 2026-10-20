// @ts-nocheck
// Guards WCAG AA (4.5:1) for small status text: the success, warning and
// error text tokens must clear 4.5:1 on every surface and on the tinted
// status backgrounds the components draw behind them, in both themes.
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";

// Tailwind palette values (hex approximations of the v4 oklch steps).
const PALETTE = {
  white: "#ffffff",
  "slate.50": "#f8fafc",
  "slate.100": "#f1f5f9",
  "slate.800": "#1e293b",
  "slate.900": "#0f172a",
  "slate.950": "#020617",
  "red.300": "#fca5a5",
  "red.400": "#f87171",
  "red.500": "#ef4444",
  "red.600": "#dc2626",
  "red.700": "#b91c1c",
  "emerald.400": "#34d399",
  "emerald.600": "#059669",
  "emerald.800": "#065f46",
  "amber.400": "#fbbf24",
  "amber.500": "#f59e0b",
  "amber.800": "#92400e",
};

const path = (p) => fileURLToPath(new URL(p, import.meta.url));
const css = readFileSync(path("../globals.css"), "utf8");

function themeBlock(selector) {
  const match = css.match(new RegExp(`^${selector} \\{([\\s\\S]*?)^\\}`, "m"));
  assert.ok(match, `${selector} block not found in globals.css`);
  return match[1];
}

function resolveToken(block, name) {
  const match = block.match(
    new RegExp(`--color-${name}:\\s*theme\\(colors\\.([a-z0-9.]+)\\);`),
  );
  assert.ok(match, `--color-${name} not found`);
  const hex = PALETTE[match[1]];
  assert.ok(hex, `no palette value for ${match[1]}`);
  return hex;
}

const rgb = (hex) =>
  [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);

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

// Every tint alpha the components put behind status text, per status
function sourceFiles(dir) {
  return readdirSync(path(dir), { withFileTypes: true }).flatMap((entry) =>
    entry.isDirectory()
      ? sourceFiles(`${dir}/${entry.name}`)
      : entry.name.endsWith(".tsx")
        ? [`${dir}/${entry.name}`]
        : [],
  );
}
const sources = sourceFiles("../components").map((f) =>
  readFileSync(path(f), "utf8"),
);
function tintAlphas(status) {
  const alphas = new Set();
  const re = new RegExp(`\\bbg-${status}(?:-bg)?/(\\d+)\\b`, "g");
  for (const src of sources) {
    for (const m of src.matchAll(re)) alphas.add(Number(m[1]) / 100);
  }
  return [...alphas];
}

for (const [label, selector] of [
  ["light", ":root"],
  ["dark", "\\.dark"],
]) {
  const block = themeBlock(selector);
  const surfaces = ["surface", "surface-raised", "surface-sunken"].map((n) =>
    resolveToken(block, n),
  );

  for (const status of ["success", "warning", "error"]) {
    test(`${status} text meets 4.5:1 in ${label} theme`, () => {
      const text = resolveToken(block, status);
      const fill = resolveToken(block, `${status}-bg`);
      const alphas = tintAlphas(status);
      for (const surface of surfaces) {
        const plain = contrast(rgb(text), rgb(surface));
        assert.ok(plain >= 4.5, `${status} on ${surface} is ${plain.toFixed(2)}:1`);
        for (const tint of [text, fill]) {
          for (const alpha of alphas) {
            const ratio = contrast(rgb(text), blend(tint, surface, alpha));
            assert.ok(
              ratio >= 4.5,
              `${status} on ${tint}/${alpha} over ${surface} is ${ratio.toFixed(2)}:1`,
            );
          }
        }
      }
    });
  }
}

test("status fills are never used as text colours", () => {
  for (const src of sources) {
    assert.doesNotMatch(src, /\btext-(?:success|warning|error)-bg\b/);
  }
});
