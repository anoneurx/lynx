//! An in-memory [`SearchIndexReader`], the reference implementation used in tests.
//!
//! It is small, deterministic, and dependency-free. It is *not* meant for production:
//! the scoring is a weighted term-frequency stand-in for BM25F, not a real implementation.
//! Its job is to let the query, ranker, and API be developed and tested against the trait
//! without the Tantivy engine in the loop.

use std::collections::{BTreeMap, BTreeSet};

use lynx_query::QueryPlan;
use time::OffsetDateTime;

use crate::reader::{
    passes_filters, Candidate, DocId, DocumentMetadata, Explanation, FieldHits, FieldWeights,
    IndexGeneration, Result, SearchIndexReader, SearchOptions, TermExplanation,
};
use crate::tokenizer::term_frequencies;

/// A document to be added to the in-memory index.
#[derive(Debug, Clone)]
pub struct IndexedDocument {
    /// Document identity.
    pub doc_id: DocId,
    /// Canonical URL.
    pub url: String,
    /// Host name, for `site:` filtering.
    pub host: String,
    /// Title text.
    pub title: String,
    /// Heading text.
    pub headings: String,
    /// Body text.
    pub body: String,
    /// Anchor text.
    pub anchors: String,
    /// Image alt text and captions.
    pub alt: String,
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
    /// Spam score.
    pub spam_score: f32,
    /// Host authority rank.
    pub host_rank: f32,
    /// Domain authority rank.
    pub domain_rank: f32,
    /// Inbound link count.
    pub link_count: u32,
    /// Duplicate cluster id.
    pub cluster_id: u64,
    /// Ranking config version.
    pub ranking_config_version: String,
}

impl IndexedDocument {
    /// A document with only the fields needed to match, everything else defaulted.
    #[must_use]
    pub fn new(
        doc_id: DocId,
        url: impl Into<String>,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        let url = url.into();
        let host = host_of(&url);
        Self {
            doc_id,
            url,
            host,
            title: title.into(),
            headings: String::new(),
            body: body.into(),
            anchors: String::new(),
            alt: String::new(),
            lang: None,
            content_type: None,
            published_at: None,
            modified_at: None,
            crawled_at: None,
            quality_score: 0.5,
            spam_score: 0.0,
            host_rank: 0.0,
            domain_rank: 0.0,
            link_count: 0,
            cluster_id: 0,
            ranking_config_version: "v0.1.0".to_string(),
        }
    }
}

/// Extract the host from a URL, best-effort.
fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(ToString::to_string))
        .unwrap_or_default()
}

/// A stored document with precomputed term statistics.
#[derive(Debug)]
struct StoredDoc {
    doc: IndexedDocument,
    title_tf: BTreeMap<String, u32>,
    headings_tf: BTreeMap<String, u32>,
    body_tf: BTreeMap<String, u32>,
    anchors_tf: BTreeMap<String, u32>,
    url_tf: BTreeMap<String, u32>,
    alt_tf: BTreeMap<String, u32>,
    all_terms: BTreeSet<String>,
    phrase_haystack: String,
    content_length: u32,
}

impl StoredDoc {
    fn build(doc: IndexedDocument) -> Self {
        let title_tf = term_frequencies(&doc.title);
        let headings_tf = term_frequencies(&doc.headings);
        let body_tf = term_frequencies(&doc.body);
        let anchors_tf = term_frequencies(&doc.anchors);
        let url_tf = term_frequencies(&doc.url);
        let alt_tf = term_frequencies(&doc.alt);

        let mut all_terms = BTreeSet::new();
        for map in [
            &title_tf,
            &headings_tf,
            &body_tf,
            &anchors_tf,
            &url_tf,
            &alt_tf,
        ] {
            all_terms.extend(map.keys().cloned());
        }
        // Phrase matching needs field order preserved, so build this from the tokens in
        // sequence rather than from the sorted term set.
        let phrase_haystack = [
            crate::tokenizer::tokenize(&doc.title),
            crate::tokenizer::tokenize(&doc.headings),
            crate::tokenizer::tokenize(&doc.body),
            crate::tokenizer::tokenize(&doc.anchors),
            crate::tokenizer::tokenize(&doc.alt),
        ]
        .concat()
        .join(" ");
        let content_length = u32::try_from(doc.body.len()).unwrap_or(u32::MAX);

        Self {
            doc,
            title_tf,
            headings_tf,
            body_tf,
            anchors_tf,
            url_tf,
            alt_tf,
            all_terms,
            phrase_haystack,
            content_length,
        }
    }
}

/// The in-memory index.
#[derive(Debug)]
pub struct InMemoryIndex {
    generation: IndexGeneration,
    docs: Vec<StoredDoc>,
}

impl InMemoryIndex {
    /// An empty index with a default generation.
    #[must_use]
    pub fn new() -> Self {
        // 2026-01-01 UTC as a fixed creation time keeps tests deterministic.
        let created_at = OffsetDateTime::from_unix_timestamp(1_767_225_600)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH);
        Self {
            generation: IndexGeneration {
                id: 0,
                tokenizer_version: "test-1".to_string(),
                ranking_config_version: "v0.1.0".to_string(),
                created_at,
                doc_count: 0,
                size_bytes: 0,
                checksum: String::new(),
                shards: 1,
            },
            docs: Vec::new(),
        }
    }

    /// Add a document.
    pub fn add(&mut self, doc: IndexedDocument) {
        self.generation.doc_count = self.generation.doc_count.saturating_add(1);
        self.docs.push(StoredDoc::build(doc));
    }

    /// Number of indexed documents.
    #[must_use]
    pub fn len(&self) -> usize {
        self.docs.len()
    }

    /// Whether the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// Score a stored document against a plan. `None` when it does not match.
    fn score_doc(
        stored: &StoredDoc,
        plan: &QueryPlan,
        opts: &SearchOptions,
    ) -> Option<(f32, FieldHits)> {
        let w = opts.weights;
        let d = &stored.doc;

        if !passes_filters(
            plan,
            &d.url,
            &d.host,
            d.lang.as_deref(),
            d.content_type.as_deref(),
            d.published_at,
            opts.now,
        ) {
            return None;
        }

        let mut score = 0.0f32;
        let mut hits = FieldHits::default();

        // must: every term present.
        for term in &plan.must {
            let t = term.text();
            if !stored.all_terms.contains(t) {
                return None;
            }
            score += accumulate(t, stored, &w, &mut hits);
        }

        // should: at least one present when this is a disjunction.
        if plan.disjunction {
            let mut matched_any = false;
            for term in &plan.should {
                let t = term.text();
                if stored.all_terms.contains(t) {
                    matched_any = true;
                    score += accumulate(t, stored, &w, &mut hits);
                }
            }
            if !matched_any {
                return None;
            }
        }

        // phrases: each must appear as adjacent terms.
        for phrase in &plan.phrases {
            let needle = format!(" {} ", phrase.text);
            if !format!(" {} ", stored.phrase_haystack).contains(&needle) {
                return None;
            }
            score += w.body;
        }

        // No positive clause and no filter is a match-all (used by counts).
        Some((score, hits))
    }
}

impl Default for InMemoryIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Accumulate a term's weighted contribution across fields.
#[allow(clippy::cast_precision_loss)] // term frequencies are tiny; f32 precision is ample
fn accumulate(term: &str, stored: &StoredDoc, w: &FieldWeights, hits: &mut FieldHits) -> f32 {
    let mut score = 0.0;
    let fields: [(&BTreeMap<String, u32>, f32, u8); 6] = [
        (&stored.title_tf, w.title, 0),
        (&stored.headings_tf, w.headings, 1),
        (&stored.body_tf, w.body, 2),
        (&stored.anchors_tf, w.anchors, 3),
        (&stored.url_tf, w.url, 4),
        (&stored.alt_tf, w.alt, 5),
    ];
    for (map, weight, which) in fields {
        if let Some(tf) = map.get(term) {
            // Saturation stand-in for BM25 term-frequency saturation.
            let saturated = (*tf as f32).min(3.0);
            score += weight * saturated;
            match which {
                0 => hits.title += *tf,
                1 => hits.headings += *tf,
                2 => hits.body += *tf,
                3 => hits.anchors += *tf,
                4 => hits.url += *tf,
                _ => hits.alt += *tf,
            }
        }
    }
    score
}

impl SearchIndexReader for InMemoryIndex {
    fn generation(&self) -> IndexGeneration {
        self.generation.clone()
    }

    fn search(&self, plan: &QueryPlan, opts: &SearchOptions) -> Result<Vec<Candidate>> {
        let mut scored: Vec<(f32, FieldHits, &StoredDoc)> = self
            .docs
            .iter()
            .filter_map(|stored| Self::score_doc(stored, plan, opts).map(|(s, h)| (s, h, stored)))
            .collect();

        scored.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then_with(|| a.2.doc.doc_id.cmp(&b.2.doc.doc_id))
        });

        Ok(scored
            .into_iter()
            .skip(opts.offset)
            .take(opts.limit)
            .map(|(score, hits, stored)| candidate_of(stored, score, hits))
            .collect())
    }

    fn count(&self, plan: &QueryPlan) -> Result<u64> {
        let n = self
            .docs
            .iter()
            .filter(|stored| Self::score_doc(stored, plan, &SearchOptions::default()).is_some())
            .count();
        Ok(u64::try_from(n).unwrap_or(u64::MAX))
    }

    fn document(&self, doc_id: DocId) -> Result<Option<DocumentMetadata>> {
        Ok(self.docs.iter().find(|s| s.doc.doc_id == doc_id).map(|s| {
            let d = &s.doc;
            DocumentMetadata {
                doc_id: d.doc_id,
                url: d.url.clone(),
                title: d.title.clone(),
                lang: d.lang.clone(),
                content_type: d.content_type.clone(),
                published_at: d.published_at,
                crawled_at: d.crawled_at,
                content_length: s.content_length,
                cluster_id: d.cluster_id,
                body: d.body.clone(),
                ranking_config_version: d.ranking_config_version.clone(),
            }
        }))
    }

    #[allow(clippy::cast_precision_loss)] // term frequencies are tiny; f32 precision is ample
    fn explain(&self, plan: &QueryPlan, doc_id: DocId) -> Result<Explanation> {
        let Some(stored) = self.docs.iter().find(|s| s.doc.doc_id == doc_id) else {
            return Ok(Explanation {
                doc_id,
                bm25: 0.0,
                terms: Vec::new(),
            });
        };
        let w = FieldWeights::default();
        let mut terms = Vec::new();
        let mut total = 0.0;
        for term in plan.must.iter().chain(&plan.should) {
            let t = term.text();
            let fields: [(&str, &BTreeMap<String, u32>, f32); 6] = [
                ("title", &stored.title_tf, w.title),
                ("headings", &stored.headings_tf, w.headings),
                ("body", &stored.body_tf, w.body),
                ("anchors", &stored.anchors_tf, w.anchors),
                ("url", &stored.url_tf, w.url),
                ("alt", &stored.alt_tf, w.alt),
            ];
            for (name, map, weight) in fields {
                if let Some(tf) = map.get(t) {
                    let contribution = weight * (*tf as f32).min(3.0);
                    total += contribution;
                    terms.push(TermExplanation {
                        term: t.to_string(),
                        field: name.to_string(),
                        tf: *tf,
                        contribution,
                    });
                }
            }
        }
        Ok(Explanation {
            doc_id,
            bm25: total,
            terms,
        })
    }
}

fn candidate_of(stored: &StoredDoc, bm25: f32, field_hits: FieldHits) -> Candidate {
    let d = &stored.doc;
    Candidate {
        doc_id: d.doc_id,
        url: d.url.clone(),
        title: d.title.clone(),
        body: d.body.clone(),
        lang: d.lang.clone(),
        content_type: d.content_type.clone(),
        published_at: d.published_at,
        modified_at: d.modified_at,
        crawled_at: d.crawled_at,
        quality_score: d.quality_score,
        spam_score: d.spam_score,
        host_rank: d.host_rank,
        domain_rank: d.domain_rank,
        content_length: stored.content_length,
        link_count: d.link_count,
        cluster_id: d.cluster_id,
        bm25,
        field_hits,
    }
}
