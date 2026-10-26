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
    passes_filters, title_prefix_phrase, Candidate, CollectionStats, DocId, DocumentMetadata,
    Explanation, FieldHits, FieldLengths, FieldWeights, IndexGeneration, Result, SearchIndexReader,
    SearchOptions, SuggestionCandidate, SuggestionKind, TermExplanation, TermFrequency,
    MIN_SUGGESTION_PREFIX_LEN,
};
use crate::tokenizer::{term_frequencies, tokenize};
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
    /// Per-field token sequences, for phrase matching. Order is the field's own order;
    /// a phrase matches only within one field, never across a field boundary.
    title_tokens: Vec<String>,
    headings_tokens: Vec<String>,
    body_tokens: Vec<String>,
    anchors_tokens: Vec<String>,
    url_tokens: Vec<String>,
    alt_tokens: Vec<String>,
    content_length: u32,
    field_lengths: FieldLengths,
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
        // Phrase matching needs per-field token order preserved, so these are built
        // from the tokens in sequence rather than from the sorted term set. The URL
        // field is included so a phrase can match there, as it does in the Tantivy
        // reader.
        let title_tokens = crate::tokenizer::tokenize(&doc.title);
        let headings_tokens = crate::tokenizer::tokenize(&doc.headings);
        let body_tokens = crate::tokenizer::tokenize(&doc.body);
        let anchors_tokens = crate::tokenizer::tokenize(&doc.anchors);
        let url_tokens = crate::tokenizer::tokenize(&doc.url);
        let alt_tokens = crate::tokenizer::tokenize(&doc.alt);
        let content_length = u32::try_from(doc.body.len()).unwrap_or(u32::MAX);

        // A field's token count is the sum of its term frequencies.
        let field_lengths = FieldLengths {
            title: sum_counts(&title_tf),
            headings: sum_counts(&headings_tf),
            body: sum_counts(&body_tf),
            anchors: sum_counts(&anchors_tf),
            url: sum_counts(&url_tf),
            alt: sum_counts(&alt_tf),
        };

        Self {
            doc,
            title_tf,
            headings_tf,
            body_tf,
            anchors_tf,
            url_tf,
            alt_tf,
            all_terms,
            title_tokens,
            headings_tokens,
            body_tokens,
            anchors_tokens,
            url_tokens,
            alt_tokens,
            content_length,
            field_lengths,
        }
    }
}

/// Sum a term-frequency map to get a field's token count.
fn sum_counts(tf: &BTreeMap<String, u32>) -> u32 {
    tf.values().copied().sum()
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
    ///
    /// The clause order mirrors the tantivy reader's query construction, so the two
    /// backends cannot drift; ~100 lines is the size of that mirroring.
    #[allow(clippy::too_many_lines)]
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

        // must: every token of every term must be present in the document, in any
        // field (tokens may land in different fields, matching the Tantivy reader's
        // per-token `any_field` clause). A term whose text yields no tokens adds no
        // requirement, exactly as the Tantivy query omits the clause.
        let mut mandatory_clause = false;
        for term in &plan.must {
            let tokens = crate::tokenizer::tokenize(term.text());
            if tokens.is_empty() {
                continue;
            }
            if !tokens.iter().all(|t| stored.all_terms.contains(t)) {
                return None;
            }
            mandatory_clause = true;
            for token in &tokens {
                score += accumulate(token, stored, &w, &mut hits);
            }
        }

        // should clauses: with a mandatory clause present they are purely additive
        // scoring; without one, at least one should-term must match so a disjunction
        // cannot widen into a match-all
        // (`docs/search/retrieval.md` § Fused retrieval).
        if plan.disjunction {
            let mut matched_any = false;
            for term in &plan.should {
                let tokens = crate::tokenizer::tokenize(term.text());
                if tokens.is_empty() {
                    continue;
                }
                if tokens.iter().all(|t| stored.all_terms.contains(t)) {
                    matched_any = true;
                    for token in &tokens {
                        score += accumulate(token, stored, &w, &mut hits);
                    }
                }
            }
            if !matched_any && plan.should.iter().any(|t| !tokenize(t.text()).is_empty()) {
                let phrase_mandatory = plan.phrases.iter().any(|p| !tokenize(&p.text).is_empty());
                let intitle_mandatory =
                    plan.filters.intitle.iter().any(|v| !tokenize(v).is_empty());
                let inanchor_mandatory = plan
                    .filters
                    .inanchor
                    .iter()
                    .any(|v| !tokenize(v).is_empty());
                if !(mandatory_clause
                    || phrase_mandatory
                    || intitle_mandatory
                    || inanchor_mandatory)
                {
                    return None;
                }
            }
        }

        // phrases: each must appear as a contiguous token run within ONE field,
        // never across a field boundary. A single-token phrase behaves like a
        // must-term; a token-less phrase adds no requirement.
        for phrase in &plan.phrases {
            let tokens = crate::tokenizer::tokenize(&phrase.text);
            if tokens.is_empty() {
                continue;
            }
            let fields: [&Vec<String>; 6] = [
                &stored.title_tokens,
                &stored.headings_tokens,
                &stored.body_tokens,
                &stored.anchors_tokens,
                &stored.url_tokens,
                &stored.alt_tokens,
            ];
            let found = if tokens.len() == 1 {
                stored.all_terms.contains(&tokens[0])
            } else {
                fields.iter().any(|seq| {
                    seq.windows(tokens.len())
                        .any(|window| window.iter().eq(tokens.iter()))
                })
            };
            if !found {
                return None;
            }
            score += w.body;
        }

        // intitle: a title-scoped requirement — every token must appear in the title, and
        // it contributes the large title weight. It never requires the term in the body.
        for value in &plan.filters.intitle {
            let tokens = crate::tokenizer::tokenize(value);
            if tokens.is_empty() {
                continue;
            }
            score += intitle_contribution(&tokens, stored, w.title, &mut hits)?;
        }

        // inanchor: anchor-text-scoped, mirroring intitle over the anchors field.
        for value in &plan.filters.inanchor {
            let tokens = crate::tokenizer::tokenize(value);
            if tokens.is_empty() {
                continue;
            }
            score += inanchor_contribution(&tokens, stored, w.anchors, &mut hits)?;
        }

        // No positive clause and no filter is a match-all (used by counts).
        Some((score, hits))
    }

    /// Number of documents containing `term` in any field.
    fn document_frequency(&self, term: &str) -> u64 {
        let n = self
            .docs
            .iter()
            .filter(|s| s.all_terms.contains(term))
            .count();
        u64::try_from(n).unwrap_or(u64::MAX)
    }

    /// The per-term, per-field frequencies the ranker needs, for the terms of `plan` that
    /// this document actually contains.
    ///
    /// Carries the negative (`must_not`) and `intitle:` terms as well: the negated ones so
    /// the ranker can detect a penalty without a second lookup, and the `intitle:` ones so
    /// their title hits reach BM25F.
    fn term_frequencies_for(&self, stored: &StoredDoc, plan: &QueryPlan) -> Vec<TermFrequency> {
        // Terms are keyed at token level: a term like `c++` never matches a token map,
        // so each term is split into its tokens and candidates carry the same keys the
        // Tantivy reader's query tokens produce.
        let mut wanted: Vec<String> = Vec::new();
        for term in plan.must.iter().chain(&plan.should).chain(&plan.must_not) {
            wanted.extend(crate::tokenizer::tokenize(term.text()));
        }
        for value in plan.filters.intitle.iter().chain(&plan.filters.inanchor) {
            wanted.extend(crate::tokenizer::tokenize(value));
        }

        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for t in wanted {
            if !seen.insert(t.clone()) {
                continue;
            }
            let tf = FieldHits {
                title: *stored.title_tf.get(t.as_str()).unwrap_or(&0),
                headings: *stored.headings_tf.get(t.as_str()).unwrap_or(&0),
                body: *stored.body_tf.get(t.as_str()).unwrap_or(&0),
                anchors: *stored.anchors_tf.get(t.as_str()).unwrap_or(&0),
                url: *stored.url_tf.get(t.as_str()).unwrap_or(&0),
                alt: *stored.alt_tf.get(t.as_str()).unwrap_or(&0),
            };
            if tf == FieldHits::default() {
                continue;
            }
            out.push(TermFrequency {
                df: self.document_frequency(t.as_str()),
                term: t,
                tf,
            });
        }
        out
    }
}

impl Default for InMemoryIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Accumulate a term's weighted contribution across fields.
#[allow(clippy::cast_precision_loss)] // term frequencies are tiny; f32 precision is ample
#[allow(clippy::too_many_arguments)] // one value type per argument keeps the call site flat
fn candidate_of(
    stored: &StoredDoc,
    bm25: f32,
    field_hits: FieldHits,
    terms: Vec<TermFrequency>,
) -> Candidate {
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
        field_lengths: stored.field_lengths,
        terms,
    }
}

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

/// Field-scoped requirement contribution: `Some` total when every token is present in
/// the field's term-frequency map, `None` otherwise. `record_hit` receives each token's
/// observed frequency.
#[allow(clippy::cast_precision_loss)] // term frequencies are tiny; f32 precision is ample
fn required_contribution(
    tokens: &[String],
    tf: &BTreeMap<String, u32>,
    weight: f32,
    mut record_hit: impl FnMut(u32),
) -> Option<f32> {
    let mut contribution = 0.0;
    for token in tokens {
        let n = tf.get(token.as_str())?;
        contribution += weight * (*n as f32).min(3.0);
        record_hit(*n);
    }
    Some(contribution)
}

/// Title-scoped contribution for an `intitle:` value.
fn intitle_contribution(
    tokens: &[String],
    stored: &StoredDoc,
    weight: f32,
    hits: &mut FieldHits,
) -> Option<f32> {
    required_contribution(tokens, &stored.title_tf, weight, |n| hits.title += n)
}

/// Anchor-text-scoped contribution for an `inanchor:` value, mirroring `intitle` over
/// the anchors field.
fn inanchor_contribution(
    tokens: &[String],
    stored: &StoredDoc,
    weight: f32,
    hits: &mut FieldHits,
) -> Option<f32> {
    required_contribution(tokens, &stored.anchors_tf, weight, |n| hits.anchors += n)
}

impl SearchIndexReader for InMemoryIndex {
    fn generation(&self) -> IndexGeneration {
        self.generation.clone()
    }

    fn collection_stats(&self) -> Result<CollectionStats> {
        Ok(CollectionStats::from_field_lengths(
            self.docs.iter().map(|d| d.field_lengths),
        ))
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
            .map(|(score, hits, stored)| {
                let terms = self.term_frequencies_for(stored, plan);
                candidate_of(stored, score, hits, terms)
            })
            .collect())
    }

    fn count(&self, plan: &QueryPlan, now: OffsetDateTime) -> Result<u64> {
        // The count honours the caller's clock so freshness and date windows behave
        // exactly as they do in `search`.
        let opts = SearchOptions {
            now,
            ..SearchOptions::default()
        };
        let n = self
            .docs
            .iter()
            .filter(|stored| Self::score_doc(stored, plan, &opts).is_some())
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

    fn suggest_candidates(&self, prefix: &str, limit: usize) -> Result<Vec<SuggestionCandidate>> {
        let prefix = prefix.trim().to_ascii_lowercase();
        if limit == 0 || prefix.len() < MIN_SUGGESTION_PREFIX_LEN {
            return Ok(Vec::new());
        }
        // Terms: document frequency per field, folded into a merged map as the max
        // across fields — the same definition the Tantivy reader uses. BTreeMap keys
        // compare byte-wise, so the tail after `prefix` is exactly the prefixed run.
        let mut per_field: [BTreeMap<String, u32>; 6] = Default::default();
        for stored in &self.docs {
            for (slot, map) in [
                &stored.title_tf,
                &stored.headings_tf,
                &stored.body_tf,
                &stored.anchors_tf,
                &stored.url_tf,
                &stored.alt_tf,
            ]
            .into_iter()
            .enumerate()
            {
                for (token, _) in map.range(prefix.clone()..) {
                    if !token.starts_with(prefix.as_str()) {
                        break;
                    }
                    *per_field[slot].entry(token.clone()).or_insert(0) += 1;
                }
            }
        }
        let mut merged_term_dfs: BTreeMap<String, u32> = BTreeMap::new();
        for field_map in &per_field {
            for (term, df) in field_map {
                merged_term_dfs
                    .entry(term.clone())
                    .and_modify(|current| *current = (*current).max(*df))
                    .or_insert(*df);
            }
        }

        let mut out: Vec<SuggestionCandidate> = merged_term_dfs
            .into_iter()
            .take(limit)
            .map(|(text, doc_count)| SuggestionCandidate {
                text,
                kind: SuggestionKind::Term,
                doc_count,
            })
            .collect();

        // Titles: the documents whose title *begins* with the prefix, one candidate per
        // distinct opening phrase.
        let mut title_counts: BTreeMap<String, u32> = BTreeMap::new();
        for stored in &self.docs {
            if let Some(phrase) = title_prefix_phrase(&stored.doc.title) {
                if phrase.starts_with(prefix.as_str()) {
                    *title_counts.entry(phrase).or_insert(0) += 1;
                }
            }
        }
        out.extend(
            title_counts
                .into_iter()
                .take(limit)
                .map(|(text, doc_count)| SuggestionCandidate {
                    text,
                    kind: SuggestionKind::Title,
                    doc_count,
                }),
        );
        Ok(out)
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

        // Positive clauses only: a negated term contributes no lexical score, and the
        // `intitle:` / `inanchor:` tokens do. Like the Tantivy reader, terms are split
        // into their tokens.
        let mut positive: Vec<String> = plan
            .must
            .iter()
            .chain(&plan.should)
            .flat_map(|t| crate::tokenizer::tokenize(t.text()))
            .collect();
        for value in plan.filters.intitle.iter().chain(&plan.filters.inanchor) {
            positive.extend(crate::tokenizer::tokenize(value));
        }

        for t in positive {
            let fields: [(&str, &BTreeMap<String, u32>, f32); 6] = [
                ("title", &stored.title_tf, w.title),
                ("headings", &stored.headings_tf, w.headings),
                ("body", &stored.body_tf, w.body),
                ("anchors", &stored.anchors_tf, w.anchors),
                ("url", &stored.url_tf, w.url),
                ("alt", &stored.alt_tf, w.alt),
            ];
            for (name, map, weight) in fields {
                if let Some(tf) = map.get(t.as_str()) {
                    let contribution = weight * (*tf as f32).min(3.0);
                    total += contribution;
                    terms.push(TermExplanation {
                        term: t.clone(),
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
