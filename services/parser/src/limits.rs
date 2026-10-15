//! Hard limits on a parse and the flags that record when one was hit.
//!
//! Every bound is a memory or CPU guarantee. Exceeding a limit is never a crash: the
//! extractor truncates and records a flag, which is a quality signal rather than silent
//! data loss (`docs/crawler/parser.md` § Hard limits).

use std::time::Duration;

/// Hard limits applied to one document.
///
/// The defaults are the v0.1 contract from `docs/crawler/parser.md`. They can be lowered
/// in tests to exercise truncation without large inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseLimits {
    /// Maximum input bytes accepted. The downloader already enforces this; the parser
    /// repeats it so a caller that bypasses the downloader cannot blow past it.
    pub max_input_bytes: usize,
    /// Maximum bytes of extracted body text retained.
    pub max_text_bytes: usize,
    /// Maximum outbound links retained, after deduplication.
    pub max_links: usize,
    /// Maximum element nesting depth tracked.
    pub max_depth: usize,
    /// Maximum element tokens processed.
    pub max_nodes: u64,
    /// Maximum bytes held for any single metadata field (title, a heading, an anchor).
    pub max_metadata_bytes: usize,
    /// Maximum JSON-LD nodes walked across all `application/ld+json` blocks.
    pub max_jsonld_nodes: u64,
    /// Maximum wall-clock time spent parsing, checked cooperatively per token.
    pub max_duration: Duration,
}

impl Default for ParseLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 5 * 1024 * 1024,
            max_text_bytes: 2 * 1024 * 1024,
            max_links: 50_000,
            max_depth: 512,
            max_nodes: 2_000_000,
            max_metadata_bytes: 64 * 1024,
            max_jsonld_nodes: 5_000,
            max_duration: Duration::from_secs(2),
        }
    }
}

impl ParseLimits {
    /// The v0.1 production limits.
    #[must_use]
    pub fn v0_1() -> Self {
        Self::default()
    }
}

/// Which limits a parse hit. A clean parse returns [`ParseFlags::default`].
// Each flag is an independent, named fact about one parse; a bitflag type would obscure
// them and lose the per-flag doc comments, so the bools are the clearer representation.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParseFlags {
    /// The input was cut at the byte cap before tokenising.
    pub input_truncated: bool,
    /// Body text was cut at the byte cap.
    pub text_truncated: bool,
    /// Outbound links were dropped at the count cap.
    pub links_truncated: bool,
    /// The element-token cap was reached.
    pub node_limit_hit: bool,
    /// The nesting-depth cap was reached.
    pub depth_limit_hit: bool,
    /// The wall-clock cap was reached and parsing stopped early.
    pub duration_limit_hit: bool,
    /// A metadata field was cut at the metadata byte cap.
    pub metadata_truncated: bool,
}

impl ParseFlags {
    /// True when any limit was hit. Maps to `ParsedDocument::truncated`.
    #[must_use]
    pub fn any(self) -> bool {
        self.input_truncated
            || self.text_truncated
            || self.links_truncated
            || self.node_limit_hit
            || self.depth_limit_hit
            || self.duration_limit_hit
            || self.metadata_truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_documented_contract() {
        let l = ParseLimits::default();
        assert_eq!(l.max_input_bytes, 5 * 1024 * 1024);
        assert_eq!(l.max_text_bytes, 2 * 1024 * 1024);
        assert_eq!(l.max_links, 50_000);
        assert_eq!(l.max_depth, 512);
        assert_eq!(l.max_nodes, 2_000_000);
        assert_eq!(l.max_metadata_bytes, 64 * 1024);
        assert_eq!(l.max_jsonld_nodes, 5_000);
        assert_eq!(l.max_duration, std::time::Duration::from_secs(2));
    }

    #[test]
    fn clean_flags_are_not_truncated() {
        assert!(!ParseFlags::default().any());
    }

    #[test]
    fn any_flag_marks_truncation() {
        let flags = ParseFlags {
            depth_limit_hit: true,
            ..ParseFlags::default()
        };
        assert!(flags.any());
    }
}
