// @ts-nocheck
// Guards the WCAG AA (4.5:1) contrast of the tertiary text token against the
// surface colour, in both light (:root) and dark (.dark) themes.
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// Tailwind v4 palette values for the steps globals.css can reference.
const PALETTE = {
  "slate.50": "#f8fafc",
  "slate.100": "#f1f5f9",
  "slate.400": "#94a3b8",
  "slate.500": "#64748b",
  "slate.800": "#1e293b",
  "slate.900": "#0f172a",
  "slate.950": "#020617",
  white: "#ffffff",
};

const css = readFileSync(
  fileURLToPath(new URL("../globals.css", import.meta.url)),
  "utf8",
);

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

function luminance(hex) {
  const channels = [1, 3, 5].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  const [r, g, b] = channels;
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

for (const [label, selector] of [
  ["light", ":root"],
  ["dark", "\\.dark"],
]) {
  test(`tertiary text meets 4.5:1 on surface in ${label} theme`, () => {
    const block = themeBlock(selector);
    const tertiary = resolveToken(block, "tertiary");
    const surface = resolveToken(block, "surface");
    assert.ok(
      contrast(tertiary, surface) >= 4.5,
      `tertiary ${tertiary} on surface ${surface} is ${contrast(tertiary, surface).toFixed(2)}:1`,
    );
  });
}
