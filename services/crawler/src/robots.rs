//! `robots.txt` parsing, policy selection, and failure classification.
//!
//! Two things are true at once here, and both matter:
//!
//! 1. `robots.txt` is a **policy** input, not a trust input. A permissive file does
//!    not buy a host extra budget and does not permit anything the safety gate
//!    refuses (see ADR-0020).
//! 2. Any failure we cannot read as explicit permission is treated as **disallow**.
//!    Assuming "no `robots.txt` after a timeout = no restrictions" would let a
//!    transient network problem silently become permission.
//!
//! The path matcher is hand-rolled and bounded. `Allow`/`Disallow` patterns are
//! attacker-controlled, so they are a regular-expression denial-of-service surface;
//! a linear scan over octets with a small star/`$` grammar avoids the backtracking
//! engine entirely.

/// The stable user-agent group token. Used for rule matching, separate from the
/// version, so a version bump never changes our `robots` group.
pub const USER_AGENT_GROUP: &str = "LynxBot";

/// The full `User-Agent` header. Identifiable, with a URL that explains what LYNX is
/// and how to reach us. LYNX never impersonates another crawler.
pub const USER_AGENT_HEADER: &str = "LynxBot/0.1 (+https://anoneurx.com/lynx/bot)";

/// A `robots.txt` larger than this is truncated and logged, not rejected.
pub const MAX_ROBOTS_BYTES: usize = 256 * 1024;

/// Line cap, so a pathological file cannot exhaust memory through line count.
pub const MAX_LINES: usize = 50_000;

/// Path-pattern length cap. Longer patterns are ignored.
pub const MAX_PATH_LEN: usize = 2_048;

/// Total rule cap across all groups.
pub const MAX_RULES: usize = 10_000;

/// Our decision for one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RobotsDecision {
    /// The path may be fetched (subject to our own policies).
    Allow,
    /// The path is disallowed by `robots.txt`.
    Disallow,
}

/// How a `robots.txt` HTTP status maps to policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RobotsStatusPolicy {
    /// 2xx: parse the body and apply the rules.
    Parse,
    /// 404/410: nothing published, allow all (subject to our own policies).
    AllowAll,
    /// 401/403 or anything uninterpretable: treat as disallow all.
    DisallowAll,
    /// 429: honour `Retry-After`, cool the host down, defer the batch.
    Cooldown,
    /// 5xx: retry with backoff; treat as disallow all until known.
    DisallowAllRetry,
}

/// Map an HTTP status to a `robots.txt` policy. Anything not explicitly permissive
/// is disallow.
#[must_use]
pub fn policy_for_status(status: u16) -> RobotsStatusPolicy {
    match status {
        200..=299 => RobotsStatusPolicy::Parse,
        404 | 410 => RobotsStatusPolicy::AllowAll,
        429 => RobotsStatusPolicy::Cooldown,
        500..=599 => RobotsStatusPolicy::DisallowAllRetry,
        // 401/403 and every other status: conservative.
        _ => RobotsStatusPolicy::DisallowAll,
    }
}

/// A compiled `Allow`/`Disallow` path pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PathMatcher {
    pattern: Vec<u8>,
    anchored: bool,
    specificity: usize,
}

impl PathMatcher {
    /// Compile `raw`, or return `None` for an empty or oversized pattern.
    fn new(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.is_empty() || raw.len() > MAX_PATH_LEN {
            return None;
        }
        let anchored = raw.ends_with('$');
        let body = if anchored { &raw[..raw.len() - 1] } else { raw };
        if body.is_empty() {
            // A bare `$` would match only the empty path; not useful as a rule.
            return None;
        }
        Some(Self {
            pattern: body.as_bytes().to_vec(),
            anchored,
            specificity: raw.len(),
        })
    }

    fn matches(&self, path: &str) -> bool {
        glob_prefix_matches(&self.pattern, self.anchored, path.as_bytes())
    }

    fn specificity(&self) -> usize {
        self.specificity
    }
}

/// Match `pattern` against `text`, anchored at the start. If `anchored_end` is set
/// (a `$` suffix), the pattern must consume the whole text; otherwise a prefix match
/// succeeds. `*` matches any sequence. Linear in `pattern.len() * text.len()` with no
/// backtracking blow-up.
fn glob_prefix_matches(pattern: &[u8], anchored_end: bool, text: &[u8]) -> bool {
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;

    while ti < text.len() {
        if pi < pattern.len() && pattern[pi] == b'*' {
            star = Some((pi + 1, ti));
            pi += 1;
        } else if pi < pattern.len() && pattern[pi] == text[ti] {
            pi += 1;
            ti += 1;
        } else if let Some((resume_pi, resume_ti)) = star {
            pi = resume_pi;
            ti = resume_ti + 1;
            star = Some((resume_pi, resume_ti + 1));
        } else {
            return false;
        }

        if !anchored_end && pi == pattern.len() {
            return true;
        }
    }

    while pi < pattern.len() && pattern[pi] == b'*' {
        pi += 1;
    }
    pi == pattern.len()
}

/// One `Allow`/`Disallow` rule.
#[derive(Debug, Clone)]
struct Rule {
    allow: bool,
    matcher: PathMatcher,
}

/// A `User-agent` block: one or more agents sharing a rule set.
#[derive(Debug, Clone, Default)]
struct Group {
    agents: Vec<String>,
    rules: Vec<Rule>,
    crawl_delay_seconds: Option<u64>,
}

impl Group {
    fn new(agent: String) -> Self {
        Self {
            agents: vec![agent],
            rules: Vec::new(),
            crawl_delay_seconds: None,
        }
    }

    fn has_content(&self) -> bool {
        !self.rules.is_empty() || self.crawl_delay_seconds.is_some()
    }
}

/// Parsed `robots.txt` for a host, independent of user agent.
#[derive(Debug, Clone, Default)]
pub struct RobotsRules {
    groups: Vec<Group>,
    sitemaps: Vec<String>,
}

/// The rules selected for one user agent, merged across matching groups.
#[derive(Debug, Clone, Default)]
pub struct GroupSelection {
    rules: Vec<Rule>,
    crawl_delay_seconds: Option<u64>,
}

impl RobotsRules {
    /// Parse `robots.txt` text.
    ///
    /// Field names are matched case-insensitively; comments (from `#`) are stripped;
    /// rules before any `User-agent` line are ignored, as the de facto standard
    /// requires.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut rules = Self::default();
        let mut current: Option<Group> = None;
        let mut total_rules = 0usize;

        for raw in text.lines().take(MAX_LINES) {
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            let Some((field, value)) = line.split_once(':') else {
                continue;
            };
            let field = field.trim().to_ascii_lowercase();
            let value = value.trim();

            match field.as_str() {
                "user-agent" => {
                    let agent = value.to_ascii_lowercase();
                    if agent.is_empty() {
                        continue;
                    }
                    let starts_new = current.as_ref().is_some_and(Group::has_content);
                    if starts_new {
                        if let Some(group) = current.take() {
                            rules.groups.push(group);
                        }
                        current = Some(Group::new(agent));
                    } else if let Some(group) = current.as_mut() {
                        group.agents.push(agent);
                    } else {
                        current = Some(Group::new(agent));
                    }
                }
                "allow" | "disallow" => {
                    let Some(group) = current.as_mut() else {
                        continue;
                    };
                    if total_rules >= MAX_RULES {
                        continue;
                    }
                    if let Some(matcher) = PathMatcher::new(value) {
                        group.rules.push(Rule {
                            allow: field == "allow",
                            matcher,
                        });
                        total_rules += 1;
                    }
                }
                "crawl-delay" => {
                    if let Some(group) = current.as_mut() {
                        if let Some(seconds) = parse_crawl_delay(value) {
                            group.crawl_delay_seconds = Some(seconds);
                        }
                    }
                }
                "sitemap" if !value.is_empty() => {
                    rules.sitemaps.push(value.to_string());
                }
                _ => {}
            }
        }

        if let Some(group) = current {
            rules.groups.push(group);
        }
        rules
    }

    /// The declared sitemap URLs, in order.
    #[must_use]
    pub fn sitemaps(&self) -> &[String] {
        &self.sitemaps
    }

    /// A policy that permits every path. Used to cache "no robots file published".
    #[must_use]
    pub fn allow_all() -> Self {
        Self::default()
    }

    /// A policy that forbids every path. Used when a host returns 401/403.
    #[must_use]
    pub fn disallow_all() -> Self {
        let mut group = Group::new("*".to_string());
        if let Some(matcher) = PathMatcher::new("/") {
            group.rules.push(Rule {
                allow: false,
                matcher,
            });
        }
        Self {
            groups: vec![group],
            sitemaps: Vec::new(),
        }
    }

    /// Select and merge the rules that apply to `user_agent`.
    ///
    /// An exact (case-insensitive) agent match wins outright. Otherwise the agent
    /// that is the longest case-insensitive substring of our token wins. Otherwise
    /// the `*` group applies. If nothing matches, no rules apply (allow all).
    #[must_use]
    pub fn select(&self, user_agent: &str) -> GroupSelection {
        let token = user_agent.to_ascii_lowercase();

        let exact: Vec<&Group> = self
            .groups
            .iter()
            .filter(|g| g.agents.iter().any(|a| a == &token))
            .collect();
        if !exact.is_empty() {
            return merge(&exact);
        }

        let best_len = self
            .groups
            .iter()
            .flat_map(|g| g.agents.iter())
            .filter(|a| a.as_str() != "*" && !a.is_empty() && token.contains(a.as_str()))
            .map(String::len)
            .max();
        if let Some(best_len) = best_len {
            let partial: Vec<&Group> = self
                .groups
                .iter()
                .filter(|g| {
                    g.agents
                        .iter()
                        .any(|a| a.len() == best_len && token.contains(a.as_str()))
                })
                .collect();
            return merge(&partial);
        }

        let wildcard: Vec<&Group> = self
            .groups
            .iter()
            .filter(|g| g.agents.iter().any(|a| a == "*"))
            .collect();
        merge(&wildcard)
    }
}

impl GroupSelection {
    /// Decide whether `path` may be fetched. The longest matching pattern wins;
    /// `Allow` wins a tie.
    ///
    /// `path` is attacker-controlled (it comes from a fetched URL), and the matcher is
    /// `O(pattern × path)`, so it is clamped to [`MAX_PATH_LEN`] first — on a UTF-8
    /// character boundary — before matching.
    #[must_use]
    pub fn decision(&self, path: &str) -> RobotsDecision {
        let path = clamp_path(path);
        let mut best: Option<&Rule> = None;
        let mut best_specificity = 0usize;

        for rule in &self.rules {
            if !rule.matcher.matches(path) {
                continue;
            }
            let specificity = rule.matcher.specificity();
            let wins = match best {
                None => true,
                Some(current) => {
                    specificity > best_specificity
                        || (specificity == best_specificity && rule.allow && !current.allow)
                }
            };
            if wins {
                best = Some(rule);
                best_specificity = specificity;
            }
        }

        match best {
            Some(rule) if !rule.allow => RobotsDecision::Disallow,
            _ => RobotsDecision::Allow,
        }
    }

    /// The effective `Crawl-delay` for this selection, if declared.
    #[must_use]
    pub fn crawl_delay_seconds(&self) -> Option<u64> {
        self.crawl_delay_seconds
    }
}

/// Parse a capped `robots.txt` body. Returns the rules and whether the body was
/// truncated.
#[must_use]
pub fn parse_capped(body: &[u8]) -> (RobotsRules, bool) {
    let truncated = body.len() > MAX_ROBOTS_BYTES;
    let slice = body.get(..MAX_ROBOTS_BYTES).unwrap_or(body);
    let text = String::from_utf8_lossy(slice);
    (RobotsRules::parse(&text), truncated)
}

/// The effective crawl delay is the larger of the site's request and our own floor.
#[must_use]
pub fn effective_crawl_delay_seconds(
    requested_seconds: Option<u64>,
    min_inter_request_seconds: u64,
) -> u64 {
    requested_seconds.map_or(min_inter_request_seconds, |r| {
        r.max(min_inter_request_seconds)
    })
}

fn merge(groups: &[&Group]) -> GroupSelection {
    let mut selection = GroupSelection::default();
    for group in groups {
        selection.rules.extend(group.rules.iter().cloned());
        if let Some(delay) = group.crawl_delay_seconds {
            selection.crawl_delay_seconds = Some(
                selection
                    .crawl_delay_seconds
                    .map_or(delay, |d| d.max(delay)),
            );
        }
    }
    selection
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(at) => &line[..at],
        None => line,
    }
}

#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)] // `value` is validated non-negative before the cast
fn parse_crawl_delay(value: &str) -> Option<u64> {
    let seconds = value.parse::<f64>().ok()?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    // Attacker-controlled: an absurd `Crawl-delay` must not pin a host out of the
    // frontier for as long as the value claims.
    let seconds = seconds.ceil() as u64;
    Some(seconds.min(MAX_CRAWL_DELAY_SECONDS))
}

/// The longest `Crawl-delay` honoured, in seconds.
pub const MAX_CRAWL_DELAY_SECONDS: u64 = 24 * 60 * 60;

/// The longest path the matcher sees: `path`, truncated to [`MAX_PATH_LEN`] bytes on a
/// character boundary.
fn clamp_path(path: &str) -> &str {
    if path.len() <= MAX_PATH_LEN {
        return path;
    }
    let mut end = MAX_PATH_LEN;
    while !path.is_char_boundary(end) {
        end -= 1;
    }
    &path[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn rules_for(text: &str) -> GroupSelection {
        RobotsRules::parse(text).select(USER_AGENT_GROUP)
    }

    #[test]
    fn disallows_a_listed_path() {
        let sel = rules_for("User-agent: *\nDisallow: /private\n");
        assert_eq!(sel.decision("/private/page"), RobotsDecision::Disallow);
        assert_eq!(sel.decision("/public"), RobotsDecision::Allow);
    }

    #[test]
    fn empty_disallow_allows_everything() {
        let sel = rules_for("User-agent: *\nDisallow:\n");
        assert_eq!(sel.decision("/anything"), RobotsDecision::Allow);
    }

    #[test]
    fn allow_overrides_a_longer_disallow_prefix() {
        let sel = rules_for("User-agent: *\nDisallow: /private/\nAllow: /private/public/\n");
        assert_eq!(sel.decision("/private/secret"), RobotsDecision::Disallow);
        assert_eq!(sel.decision("/private/public/page"), RobotsDecision::Allow);
    }

    #[test]
    fn allow_wins_a_specificity_tie() {
        let sel = rules_for("User-agent: *\nDisallow: /x\nAllow: /x\n");
        assert_eq!(sel.decision("/x"), RobotsDecision::Allow);
    }

    #[test]
    fn end_anchor_limits_the_match() {
        let sel = rules_for("User-agent: *\nDisallow: /*.pdf$\n");
        assert_eq!(sel.decision("/doc.pdf"), RobotsDecision::Disallow);
        assert_eq!(sel.decision("/doc.pdf.html"), RobotsDecision::Allow);
        assert_eq!(sel.decision("/folder/doc.pdf"), RobotsDecision::Disallow);
    }

    #[test]
    fn exact_agent_beats_wildcard() {
        let text = "User-agent: *\nDisallow: /\n\nUser-agent: LynxBot\nDisallow: /only\n";
        let sel = rules_for(text);
        assert_eq!(sel.decision("/anything"), RobotsDecision::Allow);
        assert_eq!(sel.decision("/only"), RobotsDecision::Disallow);
    }

    #[test]
    fn longest_substring_agent_wins() {
        let text = "User-agent: bot\nDisallow: /a\n\nUser-agent: lynx\nDisallow: /b\n";
        let sel = rules_for(text); // token "lynxbot" contains both "bot" and "lynx"
        assert_eq!(sel.decision("/a"), RobotsDecision::Allow);
        assert_eq!(sel.decision("/b"), RobotsDecision::Disallow);
    }

    #[test]
    fn consecutive_user_agents_share_a_block() {
        let text = "User-agent: LynxBot\nUser-agent: OtherBot\nDisallow: /shared\n";
        let sel = rules_for(text);
        assert_eq!(sel.decision("/shared"), RobotsDecision::Disallow);
    }

    #[test]
    fn directives_are_case_insensitive_and_comments_stripped() {
        let text = "USER-AGENT: *  # our group\nDISALLOW: /secret # no\n";
        let sel = rules_for(text);
        assert_eq!(sel.decision("/secret"), RobotsDecision::Disallow);
    }

    #[test]
    fn crawl_delay_and_sitemaps_are_collected() {
        let text = "User-agent: *\nCrawl-delay: 10\nSitemap: https://e.com/sitemap.xml\n";
        let parsed = RobotsRules::parse(text);
        assert_eq!(parsed.sitemaps(), ["https://e.com/sitemap.xml"]);
        assert_eq!(
            parsed.select(USER_AGENT_GROUP).crawl_delay_seconds(),
            Some(10)
        );
    }

    #[test]
    fn crawl_delay_rounds_up() {
        let parsed = RobotsRules::parse("User-agent: *\nCrawl-delay: 1.2\n");
        assert_eq!(
            parsed.select(USER_AGENT_GROUP).crawl_delay_seconds(),
            Some(2)
        );
        let invalid = RobotsRules::parse("User-agent: *\nCrawl-delay: -5\n");
        assert_eq!(invalid.select(USER_AGENT_GROUP).crawl_delay_seconds(), None);
    }

    #[test]
    fn crawl_delay_is_clamped_to_a_day() {
        let parsed = RobotsRules::parse("User-agent: *\nCrawl-delay: 999999999\n");
        assert_eq!(
            parsed.select(USER_AGENT_GROUP).crawl_delay_seconds(),
            Some(MAX_CRAWL_DELAY_SECONDS)
        );
    }

    #[test]
    fn an_overlong_path_is_clamped_before_matching() {
        let sel = rules_for("User-agent: *\nDisallow: /private\n");
        // Just over the cap, and the same path prefixed so the clamp lands mid-`/`.
        let long = format!("/private/{}", "a".repeat(MAX_PATH_LEN));
        assert_eq!(sel.decision(&long), RobotsDecision::Disallow);
        assert!(long.len() > MAX_PATH_LEN);
        // Matching works on the clamped prefix: a path whose *clamped* head is private
        // is disallowed even though the unclamped tail is not part of the pattern run.
        let mut boundary = String::from("/private/");
        while boundary.len() <= MAX_PATH_LEN {
            boundary.push('é'); // 2 bytes each, so the clamp lands mid-character
        }
        assert!(boundary.len() > MAX_PATH_LEN);
        assert_eq!(sel.decision(&boundary), RobotsDecision::Disallow);
        // And clamping never panics on any byte length up to and past the cap.
        let innocuous = format!("/public/{}", "b".repeat(MAX_PATH_LEN + 64));
        assert_eq!(sel.decision(&innocuous), RobotsDecision::Allow);
    }

    #[test]
    fn effective_delay_takes_the_larger() {
        assert_eq!(effective_crawl_delay_seconds(Some(2), 5), 5);
        assert_eq!(effective_crawl_delay_seconds(Some(30), 5), 30);
        assert_eq!(effective_crawl_delay_seconds(None, 5), 5);
    }

    #[test]
    fn no_matching_group_allows_everything() {
        let sel = rules_for("User-agent: SomeoneElse\nDisallow: /\n");
        assert_eq!(sel.decision("/x"), RobotsDecision::Allow);
    }

    #[test]
    fn rules_before_any_user_agent_are_ignored() {
        let sel = rules_for("Disallow: /\nUser-agent: *\nAllow: /\n");
        assert_eq!(sel.decision("/x"), RobotsDecision::Allow);
    }

    #[test]
    fn oversized_body_is_truncated_and_flagged() {
        let mut body = vec![b'#'; MAX_ROBOTS_BYTES + 10];
        body.extend_from_slice(b"\nUser-agent: *\nDisallow: /late\n");
        let (_rules, truncated) = parse_capped(&body);
        assert!(truncated);
    }

    #[test]
    fn status_policy_is_conservative() {
        assert_eq!(policy_for_status(200), RobotsStatusPolicy::Parse);
        assert_eq!(policy_for_status(204), RobotsStatusPolicy::Parse);
        assert_eq!(policy_for_status(404), RobotsStatusPolicy::AllowAll);
        assert_eq!(policy_for_status(410), RobotsStatusPolicy::AllowAll);
        assert_eq!(policy_for_status(403), RobotsStatusPolicy::DisallowAll);
        assert_eq!(policy_for_status(401), RobotsStatusPolicy::DisallowAll);
        assert_eq!(policy_for_status(429), RobotsStatusPolicy::Cooldown);
        assert_eq!(policy_for_status(503), RobotsStatusPolicy::DisallowAllRetry);
        assert_eq!(policy_for_status(302), RobotsStatusPolicy::DisallowAll);
    }

    #[test]
    fn pathological_patterns_terminate() {
        let pattern = format!("/*{}b$", "a*".repeat(40));
        let sel = rules_for(&format!("User-agent: *\nDisallow: {pattern}\n"));
        assert_eq!(sel.decision(&"/".repeat(200)), RobotsDecision::Allow);
    }
}
