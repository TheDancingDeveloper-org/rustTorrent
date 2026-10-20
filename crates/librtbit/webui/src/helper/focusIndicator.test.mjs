// @ts-nocheck
// Guards audit finding P1-1: a component may only remove the browser focus
// outline if the same class string puts a visible indicator back (the shared
// focus-ring utility, a focus-visible ring, or a focus background on menu
// items).
import test from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const SRC = fileURLToPath(new URL("..", import.meta.url));

function* tsxFiles(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) yield* tsxFiles(path);
    else if (entry.name.endsWith(".tsx")) yield path;
  }
}

const REPLACEMENT = /\bfocus-ring\b|focus-visible:ring|focus:ring-|focus:bg-/;

test("no class string drops the focus outline without a replacement", () => {
  const offenders = [];
  for (const file of tsxFiles(SRC)) {
    const src = readFileSync(file, "utf8");
    // Class strings are quoted or template literals on one line
    for (const [literal] of src.matchAll(/(["'`])[^"'`\n]*\1/g)) {
      if (
        literal.includes("focus:outline-none") &&
        !REPLACEMENT.test(literal)
      ) {
        offenders.push(`${file.slice(SRC.length)}: ${literal.slice(0, 80)}`);
      }
    }
  }
  assert.deepEqual(offenders, []);
});
