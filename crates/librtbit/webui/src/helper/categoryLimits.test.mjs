// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import {
  limitFieldsFromCategory,
  parseCategoryLimits,
} from "./categoryLimits.ts";

test("blank fields mean use global", () => {
  const r = parseCategoryLimits({
    ratio: " ",
    seedingMinutes: "",
    inactiveMinutes: "",
  });
  assert.deepEqual(r, { values: {} });
});

test("minutes convert to seconds", () => {
  const r = parseCategoryLimits({
    ratio: "1.5",
    seedingMinutes: "90",
    inactiveMinutes: "0",
  });
  assert.deepEqual(r, {
    values: {
      ratio_limit: 1.5,
      seeding_time_limit: 5400,
      inactive_seeding_time_limit: 0,
    },
  });
});

test("rejects bad input", () => {
  const base = { ratio: "", seedingMinutes: "", inactiveMinutes: "" };
  assert.match(
    parseCategoryLimits({ ...base, ratio: "-1" }).error,
    /0 or more/,
  );
  assert.match(parseCategoryLimits({ ...base, ratio: "abc" }).error, /number/);
  assert.match(
    parseCategoryLimits({ ...base, ratio: "Infinity" }).error,
    /number/,
  );
  assert.match(
    parseCategoryLimits({ ...base, seedingMinutes: "1.5" }).error,
    /minutes/,
  );
  assert.match(
    parseCategoryLimits({ ...base, inactiveMinutes: "-4" }).error,
    /minutes/,
  );
});

test("fields load from the daemon's duration shape", () => {
  assert.deepEqual(
    limitFieldsFromCategory({
      ratio_limit: 2,
      seeding_time_limit: { secs: 3600, nanos: 0 },
      inactive_seeding_time_limit: 120,
    }),
    { ratio: "2", seedingMinutes: "60", inactiveMinutes: "2" },
  );
  assert.deepEqual(limitFieldsFromCategory(undefined), {
    ratio: "",
    seedingMinutes: "",
    inactiveMinutes: "",
  });
});
