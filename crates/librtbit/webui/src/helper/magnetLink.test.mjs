// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import { extractTorrentLink } from "./magnetLink.ts";

const HASH = "3b245504cf5f11bbdbe1201cea6a6bf45aee1bc0";

test("accepts a bare magnet URI", () => {
  const magnet = `magnet:?xt=urn:btih:${HASH}&dn=ubuntu`;
  assert.equal(extractTorrentLink(magnet), magnet);
});

test("accepts a v2 magnet and surrounding whitespace", () => {
  const magnet = `magnet:?xt=urn:btmh:1220abcdef&dn=x`;
  assert.equal(extractTorrentLink(`  ${magnet}\n`), magnet);
});

test("picks the first usable link from a uri-list with several lines", () => {
  const text = [
    "https://example.com/page.html",
    `magnet:?xt=urn:btih:${HASH}`,
    "magnet:?xt=urn:btih:ffffffffffffffffffffffffffffffffffffffff",
  ].join("\r\n");
  assert.equal(extractTorrentLink(text), `magnet:?xt=urn:btih:${HASH}`);
});

test("accepts an HTTP(S) URL to a .torrent file", () => {
  const url = "https://tracker.example/files/linux.torrent?key=1";
  assert.equal(extractTorrentLink(url), url);
});

test("rejects a URL that is not a .torrent", () => {
  assert.equal(extractTorrentLink("https://example.com/download"), null);
});

test("rejects other schemes even when they end in .torrent", () => {
  assert.equal(extractTorrentLink("file:///home/a/linux.torrent"), null);
  assert.equal(extractTorrentLink("javascript:alert(1).torrent"), null);
});

test("rejects a magnet without an info hash", () => {
  assert.equal(extractTorrentLink("magnet:?dn=no-hash-here"), null);
});

test("returns null for plain text and empty input", () => {
  assert.equal(extractTorrentLink("hello world"), null);
  assert.equal(extractTorrentLink(""), null);
});
