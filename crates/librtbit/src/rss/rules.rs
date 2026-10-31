//! Per-feed download rules and smart-episode filters.
//!
//! Feed titles and the regexes stored against them come from untrusted input,
//! so every pattern is bounded before it is compiled and every match runs on
//! the `regex` crate, whose engine does not backtrack catastrophically.

use regex::Regex;

use super::models::RssRule;

/// Longest pattern accepted from a feed rule or a feed filter.
pub const MAX_REGEX_BYTES: usize = 256;

/// Longest title a pattern is ever matched against.
const MAX_TITLE_BYTES: usize = 1024;

/// Wall-clock budget for one match. The linear engine finishes well inside
/// this; the cap is what a caller can rely on.
const MATCH_BUDGET: std::time::Duration = std::time::Duration::from_millis(50);

/// Why a pattern was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternError {
    Empty,
    TooLong { bytes: usize },
    Invalid(String),
}

impl std::fmt::Display for PatternError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PatternError::Empty => write!(f, "regex must not be empty"),
            PatternError::TooLong { bytes } => {
                write!(f, "regex is {bytes} bytes; the limit is {MAX_REGEX_BYTES}")
            }
            PatternError::Invalid(msg) => write!(f, "invalid regex: {msg}"),
        }
    }
}

/// The regex crate's syntax errors span several lines (pattern, caret, reason).
/// Keep the reason only, so an API client can show it as one sentence.
fn one_line(err: &regex::Error) -> String {
    let text = err.to_string();
    let last = text.lines().map(str::trim).rfind(|line| !line.is_empty());
    let reason = last.unwrap_or("");
    reason.strip_prefix("error: ").unwrap_or(reason).to_owned()
}

/// A pattern that has passed the size and compile checks.
#[derive(Debug, Clone)]
pub struct BoundedPattern {
    source: String,
    compiled: Regex,
}

impl BoundedPattern {
    /// Compile `source`, refusing empty, oversized or invalid patterns.
    pub fn compile(source: &str) -> Result<Self, PatternError> {
        let source = source.trim();
        if source.is_empty() {
            return Err(PatternError::Empty);
        }
        let bytes = source.len();
        if bytes > MAX_REGEX_BYTES {
            return Err(PatternError::TooLong { bytes });
        }
        let compiled = Regex::new(source).map_err(|e| PatternError::Invalid(one_line(&e)))?;
        Ok(Self {
            source: source.to_string(),
            compiled,
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Whether `title` matches, inside the evaluation budget.
    ///
    /// A title longer than [`MAX_TITLE_BYTES`] is truncated before matching, and
    /// a match that would run past the budget is reported as no match, so a
    /// hostile title cannot stall the monitor.
    pub fn is_match(&self, title: &str) -> bool {
        let title = truncate_bytes(title, MAX_TITLE_BYTES);
        let started = std::time::Instant::now();
        let found = self.compiled.is_match(title);
        started.elapsed() <= MATCH_BUDGET && found
    }
}

fn truncate_bytes(value: &str, max: usize) -> &str {
    if value.len() <= max {
        return value;
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// A season/episode seen in a title, such as `S02E05` or `2x05`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Episode {
    pub season: u32,
    pub episode: u32,
}

/// Episodes already downloaded for one series, so a rule does not fetch the
/// same episode twice and can require "newer than the last one".
#[derive(Debug, Clone, Default)]
pub struct EpisodeTracker {
    seen: std::collections::BTreeMap<String, Vec<Episode>>,
}

impl EpisodeTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an episode as downloaded. Returns whether it was new.
    pub fn record(&mut self, series: &str, episode: Episode) -> bool {
        let episodes = self.seen.entry(normalize_series(series)).or_default();
        if episodes.contains(&episode) {
            return false;
        }
        episodes.push(episode);
        true
    }

    pub fn contains(&self, series: &str, episode: Episode) -> bool {
        self.seen
            .get(&normalize_series(series))
            .is_some_and(|episodes| episodes.contains(&episode))
    }

    /// The latest episode recorded for a series, if any.
    pub fn latest(&self, series: &str) -> Option<Episode> {
        self.seen
            .get(&normalize_series(series))
            .and_then(|episodes| episodes.iter().copied().max())
    }
}

fn normalize_series(series: &str) -> String {
    series
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

/// Pull a season and episode out of a title.
///
/// Accepts `S02E05`, `s2e5` and `2x05`. Anything else is not an episode, so a
/// rule that requires one leaves it alone.
pub fn parse_episode(title: &str) -> Option<Episode> {
    let lower = title.to_ascii_lowercase();
    if let Some(episode) = parse_tagged(&lower) {
        return Some(episode);
    }
    parse_crossed(&lower)
}

fn parse_tagged(lower: &str) -> Option<Episode> {
    let bytes = lower.as_bytes();
    let mut index = 0;
    while index + 4 < bytes.len() {
        let at_boundary = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
        if bytes[index] == b's'
            && at_boundary
            && let Some((season, after_season)) = read_number(lower, index + 1)
            && bytes.get(after_season) == Some(&b'e')
            && let Some((episode, _)) = read_number(lower, after_season + 1)
            && season > 0
            && episode > 0
        {
            return Some(Episode { season, episode });
        }
        index += 1;
    }
    None
}

fn parse_crossed(lower: &str) -> Option<Episode> {
    let bytes = lower.as_bytes();
    let mut index = 0;
    while index + 3 < bytes.len() {
        let at_boundary = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
        if bytes[index].is_ascii_digit()
            && at_boundary
            && let Some((season, after_season)) = read_number(lower, index)
            && bytes.get(after_season) == Some(&b'x')
            && bytes.get(after_season + 1).is_some_and(u8::is_ascii_digit)
            && let Some((episode, _)) = read_number(lower, after_season + 1)
            && season > 0
            && episode > 0
        {
            return Some(Episode { season, episode });
        }
        index += 1;
    }
    None
}

fn read_number(text: &str, start: usize) -> Option<(u32, usize)> {
    let bytes = text.as_bytes();
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_digit() && end - start < 4 {
        end += 1;
    }
    if end == start {
        return None;
    }
    let value: u32 = text[start..end].parse().ok()?;
    Some((value, end))
}

/// What a per-feed rule decided about one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleDecision {
    /// Download it, under this category, for the rule with this id.
    Download {
        category: Option<String>,
        rule_id: String,
        track_episodes: bool,
    },
    /// No enabled rule matched.
    Skip,
}

/// Decide whether `title` should be downloaded under the rules for one feed.
///
/// A rule matches when its pattern matches the title and, if the rule tracks
/// episodes and the title carries one, that episode has not already been
/// recorded for the series. The first matching rule wins, in the order given.
pub fn decide(rules: &[RssRule], title: &str, tracker: &EpisodeTracker) -> RuleDecision {
    let episode = parse_episode(title);
    for rule in rules.iter().filter(|rule| rule.enabled) {
        let Ok(pattern) = BoundedPattern::compile(&rule.match_regex) else {
            continue;
        };
        if !pattern.is_match(title) {
            continue;
        }
        if let Some(episode) = episode.filter(|_| rule.track_episodes) {
            let series = series_key(title);
            if tracker.contains(&series, episode) {
                return RuleDecision::Skip;
            }
        }
        return RuleDecision::Download {
            category: rule.category.clone(),
            rule_id: rule.id.clone(),
            track_episodes: rule.track_episodes,
        };
    }
    RuleDecision::Skip
}

/// The series a title belongs to: everything before the episode marker, or the
/// whole title when there is none.
pub fn series_key(title: &str) -> String {
    let lower = title.to_ascii_lowercase();
    let marker = lower
        .find(" s")
        .or_else(|| lower.find(" e"))
        .unwrap_or(lower.len());
    let crossed = find_crossed(&lower).unwrap_or(lower.len());
    normalize_series(&lower[..marker.min(crossed)])
}

fn find_crossed(lower: &str) -> Option<usize> {
    let bytes = lower.as_bytes();
    bytes.iter().enumerate().find_map(|(index, _)| {
        let at_boundary = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
        (at_boundary && parse_crossed(&lower[index..]).is_some()).then_some(index)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(pattern: &str, category: &str) -> RssRule {
        RssRule {
            id: "r1".to_string(),
            name: "rule".to_string(),
            feed_names: vec!["feed".to_string()],
            category: Some(category.to_string()),
            priority: 1,
            match_regex: pattern.to_string(),
            enabled: true,
            track_episodes: true,
        }
    }

    #[test]
    fn rejects_empty_and_oversized_patterns() {
        assert_eq!(
            BoundedPattern::compile("  ").unwrap_err(),
            PatternError::Empty
        );
        let huge = "a".repeat(MAX_REGEX_BYTES + 1);
        assert!(matches!(
            BoundedPattern::compile(&huge),
            Err(PatternError::TooLong { .. })
        ));
    }

    #[test]
    fn rejects_a_pattern_that_does_not_compile() {
        assert!(matches!(
            BoundedPattern::compile("(unclosed"),
            Err(PatternError::Invalid(_))
        ));
        let err = BoundedPattern::compile("(unclosed")
            .unwrap_err()
            .to_string();
        assert_eq!(err, "invalid regex: unclosed group");
    }

    #[test]
    fn matches_a_bounded_pattern_and_ignores_the_tail_of_a_long_title() {
        let pattern = BoundedPattern::compile(r"(?i)ubuntu").unwrap();
        assert!(pattern.is_match("Ubuntu 24.04 desktop"));
        assert!(!pattern.is_match("debian 13"));
        let long = format!("{}ubuntu", "x".repeat(MAX_TITLE_BYTES));
        assert!(!pattern.is_match(&long));
    }

    #[test]
    fn parses_season_episode_markers() {
        assert_eq!(
            parse_episode("Show.Name.S02E05.1080p"),
            Some(Episode {
                season: 2,
                episode: 5
            })
        );
        assert_eq!(
            parse_episode("Show Name - 2x15 - Title"),
            Some(Episode {
                season: 2,
                episode: 15
            })
        );
        assert_eq!(parse_episode("no episode here"), None);
    }

    #[test]
    fn a_rule_downloads_the_first_time_and_skips_a_seen_episode() {
        let rules = vec![rule(r"(?i)show name", "tv")];
        let mut tracker = EpisodeTracker::new();
        let title = "Show Name S01E02 720p";

        assert_eq!(
            decide(&rules, title, &tracker),
            RuleDecision::Download {
                category: Some("tv".to_string()),
                rule_id: "r1".to_string(),
                track_episodes: true,
            }
        );

        let episode = parse_episode(title).unwrap();
        assert!(tracker.record(&series_key(title), episode));
        assert!(!tracker.record(&series_key(title), episode));
        assert_eq!(decide(&rules, title, &tracker), RuleDecision::Skip);
    }

    #[test]
    fn a_later_episode_of_the_same_series_still_matches() {
        let rules = vec![rule(r"(?i)show name", "tv")];
        let mut tracker = EpisodeTracker::new();
        tracker.record(
            &series_key("Show Name S01E02"),
            Episode {
                season: 1,
                episode: 2,
            },
        );

        assert_eq!(
            decide(&rules, "Show Name S01E03", &tracker),
            RuleDecision::Download {
                category: Some("tv".to_string()),
                rule_id: "r1".to_string(),
                track_episodes: true,
            }
        );
        assert_eq!(
            tracker.latest("show name"),
            Some(Episode {
                season: 1,
                episode: 2
            })
        );
    }

    #[test]
    fn a_disabled_rule_never_matches() {
        let mut disabled = rule(r".*", "tv");
        disabled.enabled = false;
        assert_eq!(
            decide(&[disabled], "Show Name S01E01", &EpisodeTracker::new()),
            RuleDecision::Skip
        );
    }

    #[test]
    fn an_invalid_rule_pattern_is_skipped_not_applied() {
        let rules = vec![rule(r"(unclosed", "tv")];
        assert_eq!(
            decide(&rules, "anything", &EpisodeTracker::new()),
            RuleDecision::Skip
        );
    }

    #[test]
    fn a_rule_without_episode_tracking_downloads_a_seen_episode_again() {
        let mut untracked = rule(r"(?i)show name", "tv");
        untracked.track_episodes = false;
        let mut tracker = EpisodeTracker::new();
        let title = "Show Name S01E02 720p";
        tracker.record(&series_key(title), parse_episode(title).unwrap());

        assert_eq!(
            decide(&[untracked], title, &tracker),
            RuleDecision::Download {
                category: Some("tv".to_string()),
                rule_id: "r1".to_string(),
                track_episodes: false,
            }
        );
    }
}
