// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import {
  episodeMarker,
  matchedDate,
  ruleTracksEpisodes,
} from "./rssEpisodes.ts";

test("a rule without the field tracks episodes", () => {
  assert.equal(ruleTracksEpisodes({}), true);
  assert.equal(ruleTracksEpisodes(null), true);
  assert.equal(ruleTracksEpisodes({ track_episodes: false }), false);
  assert.equal(ruleTracksEpisodes({ track_episodes: true }), true);
});

test("episode markers are zero-padded", () => {
  assert.equal(episodeMarker({ season: 1, episode: 2 }), "S01E02");
  assert.equal(episodeMarker({ season: 12, episode: 105 }), "S12E105");
});

test("matched dates show the day only", () => {
  assert.equal(matchedDate("2026-10-31T10:00:00+00:00"), "2026-10-31");
  assert.equal(matchedDate("yesterday"), "yesterday");
});
