// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";

import {
  DEFAULT_VIEW_QUERY,
  formatViewQuery,
  parseViewQuery,
  viewsEqual,
  writeViewQuery,
} from "./viewQuery.ts";

test("parseViewQuery restores search, filters and sort", () => {
  const view = parseViewQuery(
    "?q=linux&status=seeding&category=docs&tracker=example.org&tag=linux&sort=name&dir=asc",
  );
  assert.deepEqual(view, {
    search: "linux",
    status: "seeding",
    category: "docs",
    tracker: "example.org",
    tag: "linux",
    sortColumn: "name",
    sortDirection: "asc",
  });
});

test("parseViewQuery drops unknown and invalid values", () => {
  const view = parseViewQuery("?q=&status=nope&sort=nope&dir=sideways&extra=1");
  assert.deepEqual(view, DEFAULT_VIEW_QUERY);
});

test("parseViewQuery keeps an empty tag as untagged", () => {
  const view = parseViewQuery("?tag=");
  assert.equal(view.tag, "");
});

test("formatViewQuery omits defaults and round-trips", () => {
  assert.equal(formatViewQuery(DEFAULT_VIEW_QUERY), "");
  const view = {
    ...DEFAULT_VIEW_QUERY,
    search: "a b",
    status: "downloading",
    sortColumn: "size",
    sortDirection: "asc",
  };
  const encoded = formatViewQuery(view);
  assert.equal(encoded.includes("q=a+b") || encoded.includes("q=a%20b"), true);
  assert.deepEqual(parseViewQuery(`?${encoded}`), view);
  assert.equal(viewsEqual(parseViewQuery(`?${encoded}`), view), true);
});

test("writeViewQuery replaces the query and skips a no-op", () => {
  const calls = [];
  const location = { pathname: "/ui", search: "" };
  const history = {
    replaceState: (_data, _unused, url) => calls.push(String(url)),
  };
  writeViewQuery(
    { ...DEFAULT_VIEW_QUERY, search: "iso", status: "completed" },
    location,
    history,
  );
  assert.deepEqual(calls, ["/ui?q=iso&status=completed"]);
  writeViewQuery(DEFAULT_VIEW_QUERY, { pathname: "/ui", search: "" }, history);
  assert.equal(calls.length, 1);
});
