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

test("writeViewQuery pushes a history entry and skips a no-op", () => {
  const pushed = [];
  const replaced = [];
  const location = { pathname: "/ui", search: "" };
  const history = {
    pushState: (_data, _unused, url) => pushed.push(String(url)),
    replaceState: (_data, _unused, url) => replaced.push(String(url)),
  };
  writeViewQuery(
    { ...DEFAULT_VIEW_QUERY, search: "iso", status: "completed" },
    location,
    history,
  );
  assert.deepEqual(pushed, ["/ui?q=iso&status=completed"]);
  assert.deepEqual(replaced, []);
  writeViewQuery(DEFAULT_VIEW_QUERY, { pathname: "/ui", search: "" }, history);
  assert.equal(pushed.length, 1);
});

test("writeViewQuery can replace the current entry", () => {
  const pushed = [];
  const replaced = [];
  const history = {
    pushState: (_data, _unused, url) => pushed.push(String(url)),
    replaceState: (_data, _unused, url) => replaced.push(String(url)),
  };
  writeViewQuery(
    { ...DEFAULT_VIEW_QUERY, sortColumn: "name" },
    { pathname: "/", search: "" },
    history,
    "replace",
  );
  assert.deepEqual(replaced, ["/?sort=name"]);
  assert.deepEqual(pushed, []);
});
