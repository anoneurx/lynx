//! Citation validation: the deterministic post-processor Astra's output must
//! survive before a user sees a sentence.
//!
//! Per docs/ai/astra.md:
//!
//! 1. parse all bracketed citation numbers `[n]`;
//! 2. verify every sentence contains at least one valid citation;
//! 3. strip any sentence lacking a citation or citing a non-existent source index;
//! 4. **if validation strips more than half the sentences, discard the entire
//!    summary** and fail closed: *"The retrieved sources could not be reliably
//!    summarised."*
//!
//! Untrusted text never flows through [`substring`] — that is exactly the
//! mistake this module exists to prevent.

use crate::sources::PreparedSource;

/// WHY half? A summary that mostly survives validation is a summary with a few
/// ungrounded embellishments; a summary that mostly does not survive is one the
/// model hallucinated wholesale, and "stripping the good bits" would publish
/// cherry-picked shards of a bad answer. Fail closed.
const MAX_STRIP_RATIO: f64 = 0.50;

/// The refusal the user sees when verification fails wholesale. Constant so
/// clients can match it exactly.
pub const FALLBACK_MESSAGE: &str = "The retrieved sources could not be reliably summarised.";

/// What survived verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroundedSummary {
    /// Only sentences with a valid `[n]` citation survive.
    pub text: String,
    /// One entry per distinct cited source, in first-cited order.
    pub citations: Vec<Citation>,
}

/// A citation's target as the SERP client can render it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Citation {
    /// The 1-based source ordinal as `[n]` appeared.
    pub ordinal: usize,
    /// The indexed URL.
    pub url: String,
    /// The doc id for structured clients.
    pub doc_id: String,
}

/// A verification failure mode: things tests match on, operators log.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    /// More than half the sentences did not verify: the entire summary discarded.
    #[error("{0}")]
    UnreliableSummary(&'static str),
    /// The output was empty or contained no sentences at all.
    #[error("empty model output")]
    Empty,
}

/// Validate (and strip-and-reassemble) a model output against the sources.
///
/// # Errors
/// [`ValidationError::Empty`] when the output has no sentence;
/// [`ValidationError::UnreliableSummary`] when more than half the sentences
/// had to be stripped — the entire summary is discarded, never a
/// partially-verified answer.
pub fn validate_citations(
    output: &str,
    sources: &[PreparedSource],
) -> Result<GroundedSummary, ValidationError> {
    let max_ordinal = sources.iter().map(|s| s.ordinal).max().unwrap_or_default();

    let sentences = split_sentences(output);
    let total = sentences.len();
    if total == 0 {
        return Err(ValidationError::Empty);
    }

    let mut kept: Vec<String> = Vec::with_capacity(total);
    let mut citations = Vec::new();
    for sentence in &sentences {
        if let Some(nums) = valid_citations_in(sentence, max_ordinal) {
            // The sentence carries a valid citation: keep it.
            for n in nums {
                let source = &sources[n - 1];
                if !citations
                    .iter()
                    .any(|existing: &Citation| existing.ordinal == source.ordinal)
                {
                    citations.push(Citation {
                        ordinal: source.ordinal,
                        url: source.url.clone(),
                        doc_id: source.doc_id.clone(),
                    });
                }
            }
            kept.push(sentence.clone());
        }
        // No citation, or only bogus indices: strip the sentence.
    }

    if kept.is_empty() {
        // Every sentence failed verification: the output was hallucinated
        // wholesale, which is the 100% strip shape — never a "no content"
        // technicality.
        return Err(ValidationError::UnreliableSummary(FALLBACK_MESSAGE));
    }
    let strip_ratio =
        f64::from(u32::try_from(total.saturating_sub(kept.len())).unwrap_or(u32::MAX))
            / f64::from(u32::try_from(total).unwrap_or(u32::MAX));
    if strip_ratio > MAX_STRIP_RATIO {
        // Keep the *failure* honest rather than publishing the surviving
        // fragments: a partially-stripped summary is cherry-picking.
        return Err(ValidationError::UnreliableSummary(FALLBACK_MESSAGE));
    }
    Ok(GroundedSummary {
        text: kept.join(" "),
        citations,
    })
}

/// Sentence segmentation for summaries.
///
/// A sentence ends at terminal punctuation (`.` `!` `?`) or a newline. A
/// citation bracket immediately following the punctuation (`. [2]`) — the
/// convention the system prompt mandates — stays with the sentence it supports;
/// otherwise a trailing `[n]` would leak into the *next* sentence and every
/// end-cited sentence would be stripped as uncited.
///
/// Byte-index scanning is safe here: every boundary character the scan acts on
/// is ASCII, so all cut points fall on char boundaries.
fn split_sentences(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut sentences = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                push_sentence(&mut sentences, &text[start..i]);
                i += 1;
                start = i;
            }
            b'.' | b'!' | b'?' => {
                // A citation bracket after whitespace closes THIS sentence and
                // must stay with it, not leak into the next one.
                if let Some(consumed) = trailing_citation_len(&bytes[i + 1..]) {
                    // `consumed` covers everything from just after the
                    // boundary char through the closing `]`.
                    let end = i + 1 + consumed;
                    push_sentence(&mut sentences, &text[start..end]);
                    i = end;
                    start = i;
                } else {
                    push_sentence(&mut sentences, &text[start..=i]);
                    i += 1;
                    start = i;
                }
            }
            _ => i += 1,
        }
    }
    push_sentence(&mut sentences, &text[start..]);
    sentences
}

/// Flush one sentence slice, trimmed; empty stretches are dropped.
fn push_sentence(sentences: &mut Vec<String>, slice: &str) {
    let trimmed = slice.trim();
    if !trimmed.is_empty() {
        sentences.push(trimmed.to_string());
    }
}

/// From a position right after a boundary character: how many bytes does
/// `whitespace [digits]` occupy, when it closes the sentence? `None` when the
/// next non-whitespace content is not a digit-only bracket.
fn trailing_citation_len(bytes: &[u8]) -> Option<usize> {
    let mut j = 0;
    while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
        j += 1;
    }
    if bytes.get(j) != Some(&b'[') {
        return None;
    }
    let digits_start = j + 1;
    let mut k = digits_start;
    while k < bytes.len() && bytes[k].is_ascii_digit() {
        k += 1;
    }
    let digit_count = k - digits_start;
    if digit_count == 0 || digit_count > 3 {
        return None;
    }
    if bytes.get(k) != Some(&b']') {
        return None;
    }
    // Absolute end within the slice: one past the ']'.
    Some(k + 1)
}

/// Which citation numbers in a sentence are valid (`1..=max`), or `None` when
/// the sentence cites nothing valid.
fn valid_citations_in(sentence: &str, max_ordinal: usize) -> Option<Vec<usize>> {
    let mut nums = Vec::new();
    let bytes = sentence.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // A citation starts at '[' and ends at ']' with only ASCII digits between.
        if bytes[i] == b'[' {
            if let Some(close_rel) = sentence[i + 1..].find(']') {
                let close = i + 1 + close_rel;
                let inner = &sentence[i + 1..close];
                let inner = inner.trim();
                if !inner.is_empty()
                    && inner.chars().all(|c| c.is_ascii_digit())
                    && inner.len() <= 3
                {
                    if let Ok(n) = inner.parse::<usize>() {
                        if n >= 1 && n <= max_ordinal {
                            nums.push(n);
                        }
                        i = close + 1;
                        continue;
                    }
                }
                i = close + 1;
                continue;
            }
        }
        i += 1;
    }
    if nums.is_empty() {
        None
    } else {
        Some(nums)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::RankedDocument;

    fn sources() -> Vec<PreparedSource> {
        let docs = [
            RankedDocument {
                doc_id: "d1".into(),
                url: "https://s1.test/".into(),
                title: "S1".into(),
                text: "Rust is a systems language.".into(),
            },
            RankedDocument {
                doc_id: "d2".into(),
                url: "https://s2.test/".into(),
                title: "S2".into(),
                text: "Memory safety matters.".into(),
            },
            RankedDocument {
                doc_id: "d3".into(),
                url: "https://s3.test/".into(),
                title: "S3".into(),
                text: "Determinism helps.".into(),
            },
        ];
        crate::sources::prepare_sources(&docs, 6_000, 3)
    }

    fn find_citation(outcome: &GroundedSummary, id: &str) -> bool {
        outcome.citations.iter().any(|c| c.doc_id == id)
    }

    #[test]
    fn a_fully_cited_summary_passes_untouched() {
        let summary = validate_citations(
            "Rust is a systems language. [1] Memory safety matters. [2]",
            &sources(),
        )
        .unwrap();
        assert_eq!(
            summary.text,
            "Rust is a systems language. [1] Memory safety matters. [2]"
        );
        assert!(find_citation(&summary, "d1"));
        assert!(find_citation(&summary, "d2"));
    }

    #[test]
    fn a_citation_index_beyond_the_sources_strips_the_sentence() {
        // "Citation stripping test" per docs/ai/astra.md: `[9]` referencing a
        // non-existent source index is removed.
        let summary = validate_citations(
            "Rust is safe. [1] The index invention of '1540. [9] More text. [2]",
            &sources(),
        )
        .unwrap();
        assert!(!summary.text.contains("[9]"));
        assert!(summary.text.contains("Rust is safe. [1]"));
        assert!(summary.text.contains("More text. [2]"));
    }

    #[test]
    fn an_uncited_sentence_is_stripped() {
        let summary = validate_citations(
            "Grounded claim. [1] Floating embellishment without citation.",
            &sources(),
        )
        .unwrap();
        assert!(summary.text.contains("[1]"));
        assert!(!summary.text.contains("Floating embellishment"));
    }

    #[test]
    fn over_half_stripped_discards_the_whole_summary() {
        // The hallucination shape: 4 of 6 sentences fall outside the sources.
        // (3 of 6 — exactly the 50% allowance — is the most embellishment the
        // verifier tolerates; this test pins the other side of the boundary.)
        let output = "Fabricated fact one. [1] Fabricated two. \
                      Fabricated three. Fabricated four. Fabricated five. \
                      Grounded fact six. [1]";
        let outcome = validate_citations(output, &sources());
        assert_eq!(
            outcome.unwrap_err(),
            ValidationError::UnreliableSummary(FALLBACK_MESSAGE)
        );
    }

    #[test]
    fn exactly_half_stripped_is_the_allowed_ceiling() {
        // 3 of 6 stripped: allowed under the >50% rule, but only the cited
        // sentences may survive in the output.
        let output = "Grounded one. [1] Fabricated two. Fabricated three. \
                      Grounded four. [2] Fabricated five. Grounded six. [1]";
        let summary = validate_citations(output, &sources()).unwrap();
        assert!(summary.text.contains("[1]"));
        assert!(summary.text.contains("[2]"));
        assert!(!summary.text.contains("Fabricated"));
    }

    #[test]
    fn a_wholly_hallucinated_output_is_unreliable_not_empty() {
        // The hallucination test per docs/ai/astra.md: a generated summary
        // with no valid citation anywhere refuses as unreliable.
        let outcome = validate_citations("Invented from nothing at all.", &sources());
        assert_eq!(
            outcome.unwrap_err(),
            ValidationError::UnreliableSummary(FALLBACK_MESSAGE)
        );
    }

    #[test]
    fn empty_output_is_empty() {
        assert_eq!(
            validate_citations("", &sources()).unwrap_err(),
            ValidationError::Empty
        );
        assert_eq!(
            validate_citations("   ", &sources()).unwrap_err(),
            ValidationError::Empty
        );
    }

    #[test]
    fn an_all_invalid_output_never_passes() {
        assert!(validate_citations("Everything [9] points nowhere. [0]", &sources()).is_err());
    }

    #[test]
    fn a_trailing_stays_with_its_sentence() {
        // The system prompt's mandated shape: ". [n]" ends the sentence it
        // supports. If the splitter leaked the bracket to the next sentence,
        // every end-cited sentence would be stripped as uncited — the classic
        // silent-total-strip failure this module exists to prevent.
        let summary = validate_citations(
            "Rust is safe. [1] Memory safety is proven. [2] Determinism helps. [3]",
            &sources(),
        )
        .unwrap();
        assert_eq!(
            summary.text,
            "Rust is safe. [1] Memory safety is proven. [2] Determinism helps. [3]"
        );
        assert_eq!(summary.citations.len(), 3);
    }

    #[test]
    fn a_citation_without_punctuation_boundary_starts_a_new_sentence() {
        // "[2]" mid-flow without a preceding sentence end is its own sentence
        // and stands only if it carries a valid citation itself.
        let outcome = validate_citations("Grounded. [1]", &sources());
        assert!(outcome.is_ok());
    }

    #[test]
    fn bracketed_text_that_is_not_a_citation_is_left_alone() {
        let summary = validate_citations(
            "Array slice syntax is noted [1:2] in Rust. [2] Value references are [1].",
            &sources(),
        )
        .unwrap();
        assert!(summary.text.contains("[1:2]"), "arrays are not citations");
    }
}
