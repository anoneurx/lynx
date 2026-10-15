//! Crawler-trap detection.
//!
//! A crawler trap is a URL space that generates effectively unbounded distinct URLs:
//! calendars, id ladders, session-parameter permutations, and repeated-segment loops.
//! A frontier that follows them fills the queue faster than any crawl can drain it, and
//! the "deep queue" then pushes us to crawl harder — the opposite of polite.
//!
//! Detection is a pure function of the canonical URL and a config, so it is cheap and
//! deterministic: every discovered link is classified before it is enqueued, and a trap
//! is dropped with a recorded reason rather than queued and re-discovered forever.
//!
//! These are heuristics, deliberately conservative. A false positive costs coverage of
//! one URL; a false negative costs an unbounded crawl. Where the two trade off, this
//! errs toward dropping.

use serde::{Deserialize, Serialize};
use url::Url;

/// Query parameters that carry a session identity. Their presence is a trap signal: the
/// same content is reachable under an unbounded set of URLs.
const SESSION_PARAMS: &[&str] = &[
    "sessid",
    "sessionid",
    "session",
    "sid",
    "phpsessid",
    "jsessionid",
    "aspsessionid",
    "cfid",
    "cftoken",
    "zenid",
    "oscommerceid",
];

/// Thresholds for trap detection. Mirrors the `[crawler.traps]` config block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrapConfig {
    /// Maximum characters in the whole URL.
    pub max_url_len: usize,
    /// Maximum characters in one path segment.
    pub max_segment_len: usize,
    /// Maximum number of non-empty path segments.
    pub max_path_depth: usize,
    /// Consecutive identical segments that mark a loop.
    pub max_repeated_segment: usize,
    /// Maximum query parameters.
    pub max_query_params: usize,
    /// Maximum characters in one query value.
    pub max_query_value_len: usize,
    /// Pure-numeric path segments that mark an id-ladder or calendar.
    pub max_numeric_segments: usize,
}

impl Default for TrapConfig {
    fn default() -> Self {
        Self {
            max_url_len: 4096,
            max_segment_len: 255,
            max_path_depth: 32,
            max_repeated_segment: 4,
            max_query_params: 16,
            max_query_value_len: 512,
            max_numeric_segments: 8,
        }
    }
}

/// Why a URL was classified as a trap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrapKind {
    /// The URL is longer than any content page needs to be.
    LongUrl,
    /// A single path segment is longer than any slug needs to be.
    LongSegment,
    /// The path is deeper than any real taxonomy.
    PathDepth,
    /// A segment repeats back-to-back (a path loop).
    RepeatedSegment,
    /// Too many query parameters.
    ParamExplosion,
    /// A query value is absurdly long (usually a payload or a token ladder).
    LongQueryValue,
    /// A session-identity parameter is present.
    SessionId,
    /// Too many numeric path segments (a calendar or id ladder).
    NumericDepth,
}

/// The result of classifying a URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrapVerdict {
    /// The URL looks ordinary; enqueue it.
    Ok,
    /// The URL looks like a trap; drop it with `kind`.
    Trap(TrapKind),
}

impl TrapVerdict {
    /// Whether this verdict permits the URL.
    #[must_use]
    pub fn is_ok(self) -> bool {
        matches!(self, Self::Ok)
    }

    /// The trap kind, if any.
    #[must_use]
    pub fn kind(self) -> Option<TrapKind> {
        match self {
            Self::Ok => None,
            Self::Trap(kind) => Some(kind),
        }
    }
}

/// Classify a URL against the default config.
#[must_use]
pub fn detect(url: &Url) -> TrapVerdict {
    detect_with(url, &TrapConfig::default())
}

/// Classify a URL against `config`.
///
/// Returns [`TrapVerdict::Ok`] for a URL that cannot be parsed: the caller already holds
/// a parsed URL, and an unparseable string is not, on that ground alone, a trap.
#[must_use]
pub fn detect_with(url: &Url, config: &TrapConfig) -> TrapVerdict {
    let raw = url.as_str();
    if raw.len() > config.max_url_len {
        return TrapVerdict::Trap(TrapKind::LongUrl);
    }

    let segments: Vec<&str> = url.path().split('/').filter(|s| !s.is_empty()).collect();

    if segments.len() > config.max_path_depth {
        return TrapVerdict::Trap(TrapKind::PathDepth);
    }
    if segments.iter().any(|s| s.len() > config.max_segment_len) {
        return TrapVerdict::Trap(TrapKind::LongSegment);
    }
    if has_repeated_segment(&segments, config.max_repeated_segment) {
        return TrapVerdict::Trap(TrapKind::RepeatedSegment);
    }

    let numeric = segments.iter().filter(|s| is_numeric(s)).count();
    if numeric > config.max_numeric_segments {
        return TrapVerdict::Trap(TrapKind::NumericDepth);
    }

    if let Some(kind) = classify_query(url, config) {
        return TrapVerdict::Trap(kind);
    }

    TrapVerdict::Ok
}

/// Classify a URL string, parsing it first.
#[must_use]
pub fn detect_str(raw: &str, config: &TrapConfig) -> TrapVerdict {
    match Url::parse(raw) {
        Ok(url) => detect_with(&url, config),
        Err(_) => TrapVerdict::Ok,
    }
}

fn has_repeated_segment(segments: &[&str], threshold: usize) -> bool {
    if threshold == 0 {
        return false;
    }
    let mut run = 1;
    for pair in segments.windows(2) {
        if pair[0] == pair[1] {
            run += 1;
            if run >= threshold {
                return true;
            }
        } else {
            run = 1;
        }
    }
    false
}

fn is_numeric(segment: &str) -> bool {
    !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit())
}

fn classify_query(url: &Url, config: &TrapConfig) -> Option<TrapKind> {
    let query = url.query()?;
    if query.is_empty() {
        return None;
    }

    let params: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    if params.len() > config.max_query_params {
        return Some(TrapKind::ParamExplosion);
    }
    if params
        .iter()
        .any(|(_, v)| v.len() > config.max_query_value_len)
    {
        return Some(TrapKind::LongQueryValue);
    }
    if params
        .iter()
        .any(|(k, _)| SESSION_PARAMS.contains(&k.to_ascii_lowercase().as_str()))
    {
        return Some(TrapKind::SessionId);
    }

    // Repeated parameter names create combinatorial URLs (`?tag=a&tag=b&...`).
    let mut names: Vec<String> = params.iter().map(|(k, _)| k.to_ascii_lowercase()).collect();
    names.sort_unstable();
    names.dedup();
    if names.len() < params.len() {
        return Some(TrapKind::ParamExplosion);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn classify(raw: &str) -> TrapVerdict {
        detect_with(&Url::parse(raw).unwrap(), &TrapConfig::default())
    }

    #[test]
    fn ordinary_urls_pass() {
        assert!(classify("https://e.com/").is_ok());
        assert!(classify("https://e.com/blog/2026/rust-async").is_ok());
        assert!(classify("https://e.com/search?q=rust&page=2").is_ok());
        assert!(classify("https://e.com/a/b/c/d/e").is_ok());
    }

    #[test]
    fn deep_paths_are_traps() {
        let deep = format!("https://e.com/{}", vec!["x"; 40].join("/"));
        assert_eq!(classify(&deep).kind(), Some(TrapKind::PathDepth));
    }

    #[test]
    fn repeated_segments_are_traps() {
        assert!(
            classify("https://e.com/a/b/c/d/e/f").is_ok(),
            "distinct segments are fine"
        );
        assert_eq!(
            classify("https://e.com/loop/loop/loop/loop/loop").kind(),
            Some(TrapKind::RepeatedSegment)
        );
    }

    #[test]
    fn numeric_ladders_are_traps() {
        let ladder = format!(
            "https://e.com/{}",
            (0..12).map(|n| n.to_string()).collect::<Vec<_>>().join("/")
        );
        assert_eq!(classify(&ladder).kind(), Some(TrapKind::NumericDepth));
    }

    #[test]
    fn a_calendar_like_path_is_a_trap() {
        assert_eq!(
            classify("https://e.com/2026/01/01/2026/02/02/2026/03/03/2026/04/04").kind(),
            Some(TrapKind::NumericDepth)
        );
    }

    #[test]
    fn session_parameters_are_traps() {
        assert_eq!(
            classify("https://e.com/page?PHPSESSID=abc123").kind(),
            Some(TrapKind::SessionId)
        );
        assert_eq!(
            classify("https://e.com/page?jsessionid=XYZ").kind(),
            Some(TrapKind::SessionId)
        );
    }

    #[test]
    fn parameter_explosion_is_a_trap() {
        let many = (0..20)
            .map(|n| format!("p{n}=x"))
            .collect::<Vec<_>>()
            .join("&");
        assert_eq!(
            classify(&format!("https://e.com/search?{many}")).kind(),
            Some(TrapKind::ParamExplosion)
        );
    }

    #[test]
    fn repeated_parameter_names_are_a_trap() {
        assert_eq!(
            classify("https://e.com/search?tag=a&tag=b&tag=c").kind(),
            Some(TrapKind::ParamExplosion)
        );
    }

    #[test]
    fn an_overlong_query_value_is_a_trap() {
        let value = "a".repeat(600);
        assert_eq!(
            classify(&format!("https://e.com/search?q={value}")).kind(),
            Some(TrapKind::LongQueryValue)
        );
    }

    #[test]
    fn an_overlong_url_is_a_trap() {
        let segment = "a".repeat(5000);
        assert_eq!(
            classify(&format!("https://e.com/{segment}")).kind(),
            Some(TrapKind::LongUrl)
        );
    }

    #[test]
    fn an_overlong_segment_is_a_trap() {
        let segment = "a".repeat(300);
        assert_eq!(
            classify(&format!("https://e.com/{segment}/end")).kind(),
            Some(TrapKind::LongSegment)
        );
    }

    #[test]
    fn unparseable_input_is_not_a_trap() {
        assert!(detect_str("not a url", &TrapConfig::default()).is_ok());
    }
}
