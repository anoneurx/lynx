//! The index abstraction: `SearchIndexReader` and its value types.
//!
//! This is the boundary that keeps Tantivy replaceable. Nothing in the API or the ranker
//! imports a Tantivy type; they speak only in [`Candidate`], [`DocumentMetadata`], and
//! [`Explanation`]. The trait itself pulls in no index-engine dependency, so it compiles
//! with the `tantivy` feature disabled.
//!
//! See `docs/indexing/index-schema.md` and ADR-0002.

use lynx_common::ids::DocumentId;
use lynx_query::QueryPlan;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// A document identifier.
pub type DocId = DocumentId;

/// A failed index operation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IndexError {
    /// The index is not available (not yet loaded, or all shards failed).
    #[error("index unavailable: {0}")]
    Unavailable(String),
    /// The index is present but unreadable.
    #[error("index corrupt: {0}")]
    Corrupt(String),
    /// An internal error while executing a query.
    #[error("index internal error: {0}")]
    Internal(String),
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, IndexError>;

/// Identity and provenance of an index generation.
///
/// A generation is immutable once sealed. Weights live in the reader, so a ranking weight
/// change never forces a rebuild (see `docs/indexing/index-schema.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexGeneration {
    /// Monotonic generation number.
    pub id: u64,
    /// Tokenizer/analyzer version; a change forces a rebuild.
    pub tokenizer_version: String,
    /// Ranking config version; a change does not force a rebuild.
    pub ranking_config_version: String,
    /// When the generation was sealed.
    pub created_at: OffsetDateTime,
    /// Number of documents.
    pub doc_count: u64,
    /// On-disk size in bytes.
    pub size_bytes: u64,
    /// Content checksum of the generation.
    pub checksum: String,
    /// Number of shards.
    pub shards: u32,
}

/// Reader-side field weights. Tuning these does not require a re-index.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FieldWeights {
    /// Title weight.
    pub title: f32,
    /// Headings weight.
    pub headings: f32,
    /// Anchors weight.
    pub anchors: f32,
    /// URL weight.
    pub url: f32,
    /// Body weight.
    pub body: f32,
    /// Alt-text weight.
    pub alt: f32,
}

impl Default for FieldWeights {
    fn default() -> Self {
        // From docs/indexing/index-schema.md. `body` is lowest despite carrying most text.
        Self {
            title: 12.0,
            headings: 6.0,
            anchors: 4.0,
            url: 3.0,
            body: 2.0,
            alt: 1.5,
        }
    }
}

/// Options for a single retrieval.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    /// Maximum candidates to return.
    pub limit: usize,
    /// Candidates to skip. Cursor pagination resolves this to an index offset upstream.
    pub offset: usize,
    /// The instant used for freshness and date filters. Injected for determinism.
    pub now: OffsetDateTime,
    /// Reader-side field weights.
    pub weights: FieldWeights,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            limit: 2000,
            offset: 0,
            now: OffsetDateTime::UNIX_EPOCH,
            weights: FieldWeights::default(),
        }
    }
}

/// Per-field match counts, for title boosting and explanation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldHits {
    /// Matches in the title.
    pub title: u32,
    /// Matches in headings.
    pub headings: u32,
    /// Matches in the body.
    pub body: u32,
    /// Matches in anchors.
    pub anchors: u32,
    /// Matches in the URL.
    pub url: u32,
    /// Matches in image alt text.
    pub alt: u32,
}

/// A query term's frequency in each field of one document, plus its corpus document
/// frequency.
///
/// This is the raw material BM25F needs. The reader keeps no scoring policy: it reports
/// counts, and the ranker decides what they are worth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermFrequency {
    /// The (normalised) term.
    pub term: String,
    /// Number of documents containing the term in any field.
    pub df: u64,
    /// Term frequency in each field.
    pub tf: FieldHits,
}

/// Per-field token counts of one document, for BM25F length normalisation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldLengths {
    /// Tokens in the title.
    pub title: u32,
    /// Tokens in the headings.
    pub headings: u32,
    /// Tokens in the body.
    pub body: u32,
    /// Tokens in the anchors.
    pub anchors: u32,
    /// Tokens in the URL.
    pub url: u32,
    /// Tokens in the alt text.
    pub alt: u32,
}

/// Corpus-wide average field lengths, for BM25F length normalisation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct AvgFieldLengths {
    /// Average title length, in tokens.
    pub title: f32,
    /// Average headings length, in tokens.
    pub headings: f32,
    /// Average body length, in tokens.
    pub body: f32,
    /// Average anchors length, in tokens.
    pub anchors: f32,
    /// Average URL length, in tokens.
    pub url: f32,
    /// Average alt-text length, in tokens.
    pub alt: f32,
}

/// Corpus-wide statistics a single candidate cannot carry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct CollectionStats {
    /// Number of documents in the generation.
    pub doc_count: u64,
    /// Average field lengths.
    pub avg: AvgFieldLengths,
}

impl CollectionStats {
    /// Compute corpus averages from per-document field lengths.
    ///
    /// Shared by readers so their BM25F inputs cannot drift.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // token counts fit f32 exactly
    #[allow(clippy::cast_possible_truncation)] // an average length barely spans f32's range
    pub fn from_field_lengths(lengths: impl IntoIterator<Item = FieldLengths>) -> Self {
        let mut sums = [0f64; 6];
        let mut n: u64 = 0;
        for l in lengths {
            sums[0] += f64::from(l.title);
            sums[1] += f64::from(l.headings);
            sums[2] += f64::from(l.body);
            sums[3] += f64::from(l.anchors);
            sums[4] += f64::from(l.url);
            sums[5] += f64::from(l.alt);
            n += 1;
        }
        if n == 0 {
            return Self::default();
        }
        let nf = n as f32;
        Self {
            doc_count: n,
            avg: AvgFieldLengths {
                title: sums[0] as f32 / nf,
                headings: sums[1] as f32 / nf,
                body: sums[2] as f32 / nf,
                anchors: sums[3] as f32 / nf,
                url: sums[4] as f32 / nf,
                alt: sums[5] as f32 / nf,
            },
        }
    }
}

/// A candidate document returned by retrieval, before ranking.
///
/// It carries the stored fields needed to render a result and every fast field the ranker
/// reads, so ranking never needs a second document fetch. It also carries the raw
/// per-term, per-field frequencies and its own field lengths, so BM25F can be computed
/// without revisiting the index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// Document identity.
    pub doc_id: DocId,
    /// Canonical URL.
    pub url: String,
    /// Title.
    pub title: String,
    /// Stored body text (snippet source).
    pub body: String,
    /// Language.
    pub lang: Option<String>,
    /// MIME content type.
    pub content_type: Option<String>,
    /// Publication date.
    pub published_at: Option<OffsetDateTime>,
    /// Last modified date.
    pub modified_at: Option<OffsetDateTime>,
    /// Crawl date.
    pub crawled_at: Option<OffsetDateTime>,
    /// Static quality score.
    pub quality_score: f32,
    /// Spam score in `[0,1]`.
    pub spam_score: f32,
    /// Host authority rank.
    pub host_rank: f32,
    /// Domain authority rank (capped).
    pub domain_rank: f32,
    /// Content length in bytes.
    pub content_length: u32,
    /// Inbound link count.
    pub link_count: u32,
    /// Duplicate cluster id.
    pub cluster_id: u64,
    /// Lexical relevance score from the reader (engine BM25; the ranker recomputes BM25F).
    pub bm25: f32,
    /// Per-field match counts, aggregated across the query terms.
    pub field_hits: FieldHits,
    /// Per-field token counts of this document.
    pub field_lengths: FieldLengths,
    /// Per-term, per-field frequencies for the query terms.
    pub terms: Vec<TermFrequency>,
}

/// Metadata for a single document, for the `documents/{id}` endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentMetadata {
    /// Document identity.
    pub doc_id: DocId,
    /// Canonical URL.
    pub url: String,
    /// Title.
    pub title: String,
    /// Language.
    pub lang: Option<String>,
    /// MIME content type.
    pub content_type: Option<String>,
    /// Publication date.
    pub published_at: Option<OffsetDateTime>,
    /// Crawl date.
    pub crawled_at: Option<OffsetDateTime>,
    /// Content length in bytes.
    pub content_length: u32,
    /// Duplicate cluster id.
    pub cluster_id: u64,
    /// Trailing part of the body, for the documents endpoint.
    pub body: String,
    /// The ranking config version that indexed this document.
    pub ranking_config_version: String,
}

/// Why a term contributed to a document's lexical score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TermExplanation {
    /// The (normalised) term.
    pub term: String,
    /// The field it matched in.
    pub field: String,
    /// Term frequency in that field.
    pub tf: u32,
    /// Its weighted contribution.
    pub contribution: f32,
}

/// A full, inspectable explanation of a document's lexical score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Explanation {
    /// The document explained.
    pub doc_id: DocId,
    /// Total lexical score.
    pub bm25: f32,
    /// Per-term contributions.
    pub terms: Vec<TermExplanation>,
}

/// The read side of the search index.
///
/// Implementations are immutable snapshots: a query never sees a mix of generations. The
/// in-memory [`crate::memory::InMemoryIndex`] is the reference implementation used in tests;
/// the Tantivy reader (feature `tantivy`) is the production one.
pub trait SearchIndexReader: Send + Sync {
    /// The generation this reader serves.
    fn generation(&self) -> IndexGeneration;

    /// Corpus-wide statistics for BM25F. Immutable for the life of a generation.
    ///
    /// # Errors
    /// Returns [`IndexError`] when the index cannot be read.
    fn collection_stats(&self) -> Result<CollectionStats>;

    /// Retrieve candidates matching `plan`, best-first by lexical score.
    ///
    /// # Errors
    /// Returns [`IndexError`] when the index cannot be read.
    fn search(&self, plan: &QueryPlan, opts: &SearchOptions) -> Result<Vec<Candidate>>;

    /// Count documents matching `plan`.
    ///
    /// `now` is the instant used for freshness and date filters, so a count agrees with
    /// a search run under the same clock.
    ///
    /// # Errors
    /// Returns [`IndexError`] when the index cannot be read.
    fn count(&self, plan: &QueryPlan, now: OffsetDateTime) -> Result<u64>;

    /// Fetch metadata for a document.
    ///
    /// # Errors
    /// Returns [`IndexError`] when the index cannot be read.
    fn document(&self, doc_id: DocId) -> Result<Option<DocumentMetadata>>;

    /// Explain the lexical score of a document for `plan`.
    ///
    /// # Errors
    /// Returns [`IndexError`] when the index cannot be read.
    fn explain(&self, plan: &QueryPlan, doc_id: DocId) -> Result<Explanation>;
}

/// Apply the hard filters of `plan` to a candidate's fields, returning whether it passes.
///
/// Shared by implementations so filter semantics cannot drift between the fake and the
/// Tantivy reader.
#[must_use]
pub fn passes_filters(
    plan: &QueryPlan,
    url: &str,
    host: &str,
    lang: Option<&str>,
    content_type: Option<&str>,
    published_at: Option<OffsetDateTime>,
    now: OffsetDateTime,
) -> bool {
    let f = &plan.filters;

    if let Some(site) = &f.site {
        let host_l = host.to_ascii_lowercase();
        if !(host_l == *site || host_l.ends_with(&format!(".{site}"))) {
            return false;
        }
    }
    if let Some(inurl) = &f.inurl {
        if !url.contains(inurl.as_str()) {
            return false;
        }
    }
    if !f.ext.is_empty() {
        let url_l = url.to_ascii_lowercase();
        if !f.ext.iter().any(|e| url_l.ends_with(&format!(".{e}"))) {
            return false;
        }
    }
    if !f.filetype.is_empty() {
        let ct = content_type.unwrap_or("").to_ascii_lowercase();
        if !f
            .filetype
            .iter()
            .any(|ft| ct.contains(ft.as_str()) || ct.ends_with(ft.as_str()))
        {
            return false;
        }
    }
    if let Some(lang_req) = &f.lang {
        let Some(l) = lang else { return false };
        if !l.eq_ignore_ascii_case(lang_req) {
            return false;
        }
    }
    if f.before.is_some() || f.after.is_some() {
        let Some(date) = published_at else {
            // A date filter is an explicit request; a document without a date cannot
            // satisfy it and is excluded.
            return false;
        };
        let day = date.date();
        if let Some(before) = f.before {
            if day >= before {
                return false;
            }
        }
        if let Some(after) = f.after {
            if day < after {
                return false;
            }
        }
    }
    if let Some(window) = f.freshness {
        // A document without a date cannot satisfy a recency window.
        let Some(date) = published_at else {
            return false;
        };
        let cutoff = now - time::Duration::seconds(window.approx_seconds());
        if date < cutoff {
            return false;
        }
    }
    true
}
