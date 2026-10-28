// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";

import {
  appendSpeedSample,
  polylineAttr,
  speedPolyline,
  speedScale,
} from "./speedHistory.ts";

test("appendSpeedSample keeps the newest samples up to capacity", () => {
  let history = [];
  history = appendSpeedSample(history, { downMbps: 1, upMbps: 2 }, 2);
  history = appendSpeedSample(history, { downMbps: 3, upMbps: 4 }, 2);
  history = appendSpeedSample(history, { downMbps: 5, upMbps: 6 }, 2);
  assert.deepEqual(history, [
    { downMbps: 3, upMbps: 4 },
    { downMbps: 5, upMbps: 6 },
  ]);
});

test("appendSpeedSample treats a bad reading as idle", () => {
  const history = appendSpeedSample(
    [],
    { downMbps: Number.NaN, upMbps: -3 },
    4,
  );
  assert.deepEqual(history, [{ downMbps: 0, upMbps: 0 }]);
});

test("speedScale is the peak and never below 1", () => {
  assert.equal(speedScale([]), 1);
  assert.equal(
    speedScale([
      { downMbps: 0.2, upMbps: 4 },
      { downMbps: 8, upMbps: 1 },
    ]),
    8,
  );
});

test("speedPolyline puts the newest sample on the right and idle at the bottom", () => {
  const points = speedPolyline(
    [
      { downMbps: 0, upMbps: 0 },
      { downMbps: 10, upMbps: 0 },
    ],
    "downMbps",
    100,
    40,
    10,
  );
  assert.deepEqual(points, [
    { x: 0, y: 40 },
    { x: 100, y: 0 },
  ]);
  assert.equal(polylineAttr(points), "0,40 100,0");
});
