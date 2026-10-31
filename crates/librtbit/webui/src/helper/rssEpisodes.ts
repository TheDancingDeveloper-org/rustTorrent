import type { RssRule, RssRuleEpisode } from "../api-types";

/** A rule tracks episodes unless the daemon says otherwise (older daemons omit it). */
export const ruleTracksEpisodes = (
  rule?: Pick<RssRule, "track_episodes"> | null,
) => rule?.track_episodes ?? true;

/** `S01E02`, zero-padded like release names. */
export const episodeMarker = (
  entry: Pick<RssRuleEpisode, "season" | "episode">,
) =>
  `S${String(entry.season).padStart(2, "0")}E${String(entry.episode).padStart(2, "0")}`;

/** The date part of `matched_at`, or the raw value when it is not ISO-shaped. */
export const matchedDate = (matchedAt: string) =>
  /^\d{4}-\d{2}-\d{2}/.test(matchedAt) ? matchedAt.slice(0, 10) : matchedAt;
