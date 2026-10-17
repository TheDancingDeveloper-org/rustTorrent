// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import { retryDelayMs, RETRY_MAX_MS, RETRY_MIN_MS } from "./retryBackoff.ts";

test("first failure waits the minimum delay", () => {
  assert.equal(retryDelayMs(1), RETRY_MIN_MS);
});

test("delay doubles with each consecutive failure", () => {
  assert.deepEqual([1, 2, 3, 4].map(retryDelayMs), [1000, 2000, 4000, 8000]);
});

test("delay is capped at the maximum", () => {
  assert.equal(retryDelayMs(6), RETRY_MAX_MS);
  assert.equal(retryDelayMs(50), RETRY_MAX_MS);
  assert.equal(retryDelayMs(10_000), RETRY_MAX_MS);
});

test("zero or negative failure counts give the minimum delay", () => {
  assert.equal(retryDelayMs(0), RETRY_MIN_MS);
  assert.equal(retryDelayMs(-3), RETRY_MIN_MS);
});
