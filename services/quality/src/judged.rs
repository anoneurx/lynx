//! Judged query sets: query → relevance labels over documents identified by their
//! canonical URL.
//!
//! Format (JSONL, one query per line), matching the schema documented in
//! `docs/testing/quality-evaluation.md`, keyed by canonical URL rather than opaque id:
//!
//! ```json
//! {"query": "rust async", "judgments": [{"doc": "https://tokio.rs/", "relevance": 3}]}
//! ```
//!
//! Labels use the 0–3 vocabulary: 0 not relevant, 1 marginally relevant, 2 relevant,
//! 3 the result a person would have wanted. Parsing is strict: a malformed line is a
//! data bug, never a silently skipped query.

use serde::Deserialize;

/// One judged query.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct JudgedQuery {
    /// The query exactly as it would be typed.
    pub query: String,
    /// Relevance labels for the documents the annotator inspected.
    #[serde(default)]
    pub judgments: Vec<Judgment>,
}

/// One document's relevance label within a judged query.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Judgment {
    /// Canonical URL of the judged document.
    pub doc: String,
    /// Relevance, 0–3.
    pub relevance: u8,
}

/// Why a judged set cannot be loaded.
#[derive(Debug, thiserror::Error)]
pub enum JudgedSetError {
    /// A line was not valid JSON.
    #[error("judged set line {1}: {0}")]
    Malformed(#[source] serde_json::Error, usize),
    /// A structurally valid line was semantically wrong.
    #[error("judged set line {1}: {0}")]
    Invalid(&'static str, usize),
}

impl Judgment {
    /// Whether the label counts as relevant for recall/MRR (relevance ≥ 1).
    #[must_use]
    pub fn is_relevant(&self) -> bool {
        self.relevance >= 1
    }
}

/// Parse a judged set (one JSON object per non-empty line).
///
/// # Errors
/// [`JudgedSetError::Malformed`] for JSON that does not parse; [`JudgedSetError::Invalid`]
/// for an empty query, a relevance above 3, or two judgments for the same document.
pub fn parse_jsonl(text: &str) -> Result<Vec<JudgedQuery>, JudgedSetError> {
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line_no = index + 1;
        let q: JudgedQuery =
            serde_json::from_str(line).map_err(|e| JudgedSetError::Malformed(e, line_no))?;

        if q.query.trim().is_empty() {
            return Err(JudgedSetError::Invalid("empty query", line_no));
        }
        if q.judgments.iter().any(|j| j.relevance > 3) {
            return Err(JudgedSetError::Invalid("relevance above 3", line_no));
        }
        let mut seen: Vec<&str> = Vec::with_capacity(q.judgments.len());
        for j in &q.judgments {
            if seen.contains(&j.doc.as_str()) {
                return Err(JudgedSetError::Invalid(
                    "duplicate judged document",
                    line_no,
                ));
            }
            seen.push(j.doc.as_str());
        }
        out.push(q);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lines_and_strips_comments_and_blanks() {
        let text = "# v0 judge set\n\
            {\"query\": \"rust async\", \"judgments\": [{\"doc\": \"https://a/\", \"relevance\": 3}]}\n\
            \n\
            {\"query\": \"sqlite\", \"judgments\": []}\n";
        let qs = parse_jsonl(text).unwrap();
        assert_eq!(qs.len(), 2);
        assert_eq!(qs[0].judgments[0].doc, "https://a/");
        assert_eq!(qs[0].judgments[0].relevance, 3);
        assert!(qs[1].judgments.is_empty());
    }

    #[test]
    fn relevance_is_a_0_to_3_vocabulary() {
        let text =
            "{\"query\": \"x\", \"judgments\": [{\"doc\": \"https://a/\", \"relevance\": 4}]}";
        let err = parse_jsonl(text).unwrap_err();
        assert!(matches!(
            err,
            JudgedSetError::Invalid("relevance above 3", 1)
        ));
    }

    #[test]
    fn a_document_is_judged_at_most_once_per_query() {
        let text =
            "{\"query\": \"x\", \"judgments\": [{\"doc\": \"https://a/\", \"relevance\": 2}, \
            {\"doc\": \"https://a/\", \"relevance\": 3}]}";
        let err = parse_jsonl(text).unwrap_err();
        assert!(matches!(
            err,
            JudgedSetError::Invalid("duplicate judged document", 1)
        ));
    }

    #[test]
    fn empty_queries_are_rejected() {
        let err = parse_jsonl("{\"query\": \"   \", \"judgments\": []}").unwrap_err();
        assert!(matches!(err, JudgedSetError::Invalid("empty query", _)));
    }

    #[test]
    fn malformed_json_is_reported_with_the_line_number() {
        let err = parse_jsonl("{\"query\": \"x\", nope").unwrap_err();
        assert!(matches!(err, JudgedSetError::Malformed(_, 1)));
    }
}
