//! Unicode-safe text handling.
//!
//! All user-facing and crawled strings pass through here before being logged,
//! indexed, or rendered. The goals are: canonical form, no invisible characters that
//! can smuggle instructions or break logs, and truncation that never splits a
//! grapheme cluster.

use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

/// Normalise to NFC (composed). Use for storage and comparison.
#[must_use]
pub fn nfc(input: &str) -> String {
    input.nfc().collect()
}

/// Normalise to NFKC (compatibility-composed). Use for search input, where folding
/// full-width and other compatibility forms helps matching.
#[must_use]
pub fn nfkc(input: &str) -> String {
    input.nfkc().collect()
}

/// True for control, zero-width, bidi-override, and other default-ignorable code
/// points that should not survive input processing.
///
/// This is deliberately conservative: it removes characters that are invisible in
/// most renderers but can reorder text or carry hidden payloads.
#[must_use]
pub fn is_invisible(c: char) -> bool {
    matches!(c,
        // C0/C1 controls, excluding tab/newline handled by callers as needed
        '\u{0000}'..='\u{0008}'
        | '\u{000B}'..='\u{000C}'
        | '\u{000E}'..='\u{001F}'
        | '\u{007F}'..='\u{009F}'
        // Zero-width and word joiner
        | '\u{200B}'..='\u{200F}'
        | '\u{2060}'..='\u{2064}'
        | '\u{FEFF}'
        // Bidi controls
        | '\u{202A}'..='\u{202E}'
        | '\u{2066}'..='\u{2069}'
        // Soft hyphen
        | '\u{00AD}'
    )
}

/// Strip invisible/control characters, replacing runs of whitespace with a single
/// space and trimming the ends. Newlines are collapsed like any whitespace.
#[must_use]
pub fn clean(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_space = true; // start true to trim leading space
    for c in input.chars() {
        if is_invisible(c) {
            continue;
        }
        if c.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(c);
            last_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Truncate to at most `max_chars` grapheme clusters, appending `…` if truncated.
///
/// Grapheme-aware so it never splits a combining sequence or emoji.
#[must_use]
pub fn truncate_graphemes(input: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (count, grapheme) in input.graphemes(true).enumerate() {
        if count >= max_chars {
            out.push('…');
            return out;
        }
        out.push_str(grapheme);
    }
    out
}

/// Fold width and case for comparison keys (does not touch diacritics).
#[must_use]
pub fn fold_for_compare(input: &str) -> String {
    nfkc(input).to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nfc_composes() {
        // "e" + combining acute -> single composed code point.
        let decomposed = "e\u{0301}";
        assert_eq!(nfc(decomposed).chars().count(), 1);
    }

    #[test]
    fn nfkc_folds_fullwidth() {
        assert_eq!(nfkc("ＲＵＳＴ"), "RUST");
    }

    #[test]
    fn clean_removes_invisible_and_collapses_space() {
        let input = "a\u{200B}b   c\n\td";
        assert_eq!(clean(input), "ab c d");
    }

    #[test]
    fn clean_strips_bidi_override() {
        let input = "safe\u{202E}evil";
        assert_eq!(clean(input), "safeevil");
    }

    #[test]
    fn truncate_is_grapheme_safe() {
        // Family emoji is one grapheme made of several code points.
        let family = "👩‍👩‍👧‍👦";
        let s = format!("ab{family}cd");
        let out = truncate_graphemes(&s, 3);
        assert_eq!(out, format!("ab{family}…"));
    }

    #[test]
    fn truncate_without_cut_adds_nothing() {
        assert_eq!(truncate_graphemes("abc", 5), "abc");
    }

    #[test]
    fn fold_is_case_and_width_insensitive() {
        assert_eq!(fold_for_compare("ＦＯＯ"), "foo");
    }
}
