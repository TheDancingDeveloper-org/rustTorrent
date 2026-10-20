// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";

import { nextSort } from "./sortToggle.ts";

test("a new column sorts descending", () => {
  assert.deepEqual(nextSort({ column: "name", direction: "asc" }, "size"), {
    column: "size",
    direction: "desc",
  });
});

test("the sorted column flips its direction each time", () => {
  let state = { column: "name", direction: "desc" };
  state = nextSort(state, "name");
  assert.deepEqual(state, { column: "name", direction: "asc" });
  state = nextSort(state, "name");
  assert.deepEqual(state, { column: "name", direction: "desc" });
});

test("does not mutate the previous state", () => {
  const prev = Object.freeze({ column: "name", direction: "asc" });
  nextSort(prev, "name");
  assert.deepEqual(prev, { column: "name", direction: "asc" });
});
