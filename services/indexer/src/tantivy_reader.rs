//! A Tantivy-backed [`SearchIndexReader`], available behind the `tantivy` feature.
//!
//! Tantivy types are confined to this module. The rest of the workspace sees only the
//! trait and its value types, so the engine stays replaceable (see ADR-0002). This is the
//! production reader; the in-memory [`crate::memory::InMemoryIndex`] is the reference.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::path::Path;

use lynx_query::QueryPlan;
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{AllQuery, BooleanQuery, Occur, PhraseQuery, Query, RangeQuery, TermQuery};
use tantivy::schema::document::Value as _;
use tantivy::schema::{
    Field, IndexRecordOption, NumericOptions, Schema, TantivyDocument, Term, TextFieldIndexing,
    TextOptions, STORED, STRING,
};
use tantivy::tokenizer::{
    Token as TvToken, TokenStream as TvTokenStream, Tokenizer as TvTokenizer,
};
use tantivy::{Index, IndexReader, Searcher};
use time::OffsetDateTime;

use crate::memory::IndexedDocument;
use crate::reader::{
    passes_filters, Candidate, CollectionStats, DocId, DocumentMetadata, Explanation, FieldHits,
    FieldLengths, FieldWeights, IndexError, IndexGeneration, Result, SearchIndexReader,
    SearchOptions, TermExplanation, TermFrequency,
};
use crate::tokenizer::{tokenize, tokenize_with_spans};

/// The manifest file written alongside a generation's index segments.
const GENERATION_FILE: &str = "generation.json";

/// The name under which the LYNX tokenizer is registered, matching the schema's
/// `TextFieldIndexing` tokenizers so index-time analysis equals query-time analysis
/// (`docs/indexing/index-schema.md` § Analyzers).
const LYNX_TOKENIZER: &str = "lynx";

/// Text-field options that use the LYNX tokenizer and store positions (phrases need
/// them), replacing tantivy's default `TEXT` options.
fn text_field_options() -> TextOptions {
    TextOptions::default().set_stored().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(LYNX_TOKENIZER)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    )
}

/// The `published_at` field needs a fast field so date ranges can be pushed down as a
/// `RangeQuery` instead of a full-document scan.
fn timestamp_options() -> NumericOptions {
    NumericOptions::default()
        .set_stored()
        .set_indexed()
        .set_fast()
}

/// The concrete schema, resolved to field handles once.
struct Fields {
    doc_id: Field,
    url: Field,
    title: Field,
    headings: Field,
    body: Field,
    anchors: Field,
    alt: Field,
    host: Field,
    lang: Field,
    content_type: Field,
    published_at: Field,
    modified_at: Field,
    crawled_at: Field,
    quality_score: Field,
    spam_score: Field,
    host_rank: Field,
    domain_rank: Field,
    content_length: Field,
    link_count: Field,
    cluster_id: Field,
    ranking_config_version: Field,
}

impl Fields {
    /// The full-text fields a lexical term may match in, in weight order.
    fn text(&self) -> [Field; 6] {
        [
            self.title,
            self.headings,
            self.anchors,
            self.url,
            self.alt,
            self.body,
        ]
    }
}

/// A sealed, growable-until-committed Tantivy index exposed through the LYNX trait.
pub struct TantivyIndexReader {
    reader: IndexReader,
    fields: Fields,
    generation: IndexGeneration,
    /// Corpus averages, computed once from the segments' inverted indexes and cached for
    /// the lifetime of this reader ([`TantivyIndexReader`] is read-only: a new generation
    /// means a new instance, so the cache cannot go stale).
    stats: std::sync::OnceLock<CollectionStats>,
}

impl std::fmt::Debug for TantivyIndexReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TantivyIndexReader")
            .field("generation", &self.generation.id)
            .finish_non_exhaustive()
    }
}

#[allow(clippy::expect_used)] // the schema is built by `build_schema`, so every lookup succeeds
fn build_fields(schema: &Schema) -> Fields {
    Fields {
        doc_id: schema.get_field("doc_id").expect("schema is built by us"),
        url: schema.get_field("url").expect("schema is built by us"),
        title: schema.get_field("title").expect("schema is built by us"),
        headings: schema.get_field("headings").expect("schema is built by us"),
        body: schema.get_field("body").expect("schema is built by us"),
        anchors: schema.get_field("anchors").expect("schema is built by us"),
        alt: schema.get_field("alt").expect("schema is built by us"),
        host: schema.get_field("host").expect("schema is built by us"),
        lang: schema.get_field("lang").expect("schema is built by us"),
        content_type: schema
            .get_field("content_type")
            .expect("schema is built by us"),
        published_at: schema
            .get_field("published_at")
            .expect("schema is built by us"),
        modified_at: schema
            .get_field("modified_at")
            .expect("schema is built by us"),
        crawled_at: schema
            .get_field("crawled_at")
            .expect("schema is built by us"),
        quality_score: schema
            .get_field("quality_score")
            .expect("schema is built by us"),
        spam_score: schema
            .get_field("spam_score")
            .expect("schema is built by us"),
        host_rank: schema
            .get_field("host_rank")
            .expect("schema is built by us"),
        domain_rank: schema
            .get_field("domain_rank")
            .expect("schema is built by us"),
        content_length: schema
            .get_field("content_length")
            .expect("schema is built by us"),
        link_count: schema
            .get_field("link_count")
            .expect("schema is built by us"),
        cluster_id: schema
            .get_field("cluster_id")
            .expect("schema is built by us"),
        ranking_config_version: schema
            .get_field("ranking_config_version")
            .expect("schema is built by us"),
    }
}

fn build_schema() -> (Schema, Fields) {
    let mut b = Schema::builder();
    b.add_text_field("doc_id", STRING | STORED);
    b.add_text_field("url", text_field_options());
    b.add_text_field("title", text_field_options());
    b.add_text_field("headings", text_field_options());
    b.add_text_field("body", text_field_options());
    b.add_text_field("anchors", text_field_options());
    b.add_text_field("alt", text_field_options());
    b.add_text_field("host", STRING | STORED);
    b.add_text_field("lang", STRING | STORED);
    b.add_text_field("content_type", STRING | STORED);
    b.add_i64_field("published_at", timestamp_options());
    b.add_i64_field("modified_at", STORED);
    b.add_i64_field("crawled_at", STORED);
    b.add_f64_field("quality_score", STORED);
    b.add_f64_field("spam_score", STORED);
    b.add_f64_field("host_rank", STORED);
    b.add_f64_field("domain_rank", STORED);
    b.add_u64_field("content_length", STORED);
    b.add_u64_field("link_count", STORED);
    b.add_u64_field("cluster_id", STORED);
    b.add_text_field("ranking_config_version", STRING | STORED);
    let schema = b.build();
    let fields = build_fields(&schema);
    (schema, fields)
}

/// Adapter exposing the LYNX tokenizer to Tantivy: index-time analysis is exactly
/// `lynx_indexer::tokenizer::tokenize`, including its NFKC/case folding, so a query token
/// matches the same surface form at index time (`docs/indexing/index-schema.md`).
#[derive(Debug, Clone, Copy)]
struct LynxTokenizer;

impl TvTokenizer for LynxTokenizer {
    type TokenStream<'a> = LynxTokenStream;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        // Spans are byte offsets within the folded text: best-effort offset info, as the
        // fold itself changes lengths.
        let tokens = tokenize_with_spans(text)
            .into_iter()
            .enumerate()
            .map(|(position, span)| TvToken {
                offset_from: span.start,
                offset_to: span.end,
                position,
                text: span.text,
                position_length: 1,
            })
            .collect();
        LynxTokenStream {
            tokens,
            next: 0,
            current: TvToken::default(),
        }
    }
}

/// A [`TvTokenStream`] over a pre-tokenised LYNX token list.
#[derive(Debug)]
struct LynxTokenStream {
    tokens: Vec<TvToken>,
    next: usize,
    current: TvToken,
}

impl TvTokenStream for LynxTokenStream {
    fn advance(&mut self) -> bool {
        if self.next >= self.tokens.len() {
            return false;
        }
        self.current = self.tokens[self.next].clone();
        self.next += 1;
        true
    }

    fn token(&self) -> &TvToken {
        &self.current
    }

    fn token_mut(&mut self) -> &mut TvToken {
        &mut self.current
    }
}

/// Register the LYNX tokenizer on `index`. Must be called before documents are added or
/// a reader is opened, whichever comes first.
fn register_lynx_tokenizer(index: &Index) {
    index.tokenizers().register(LYNX_TOKENIZER, LynxTokenizer);
}

impl TantivyIndexReader {
    /// Build an in-RAM index from `docs` and expose it as a reader.
    ///
    /// # Errors
    /// Returns [`IndexError`] if the Tantivy index cannot be created or committed.
    pub fn from_documents(
        docs: impl IntoIterator<Item = IndexedDocument>,
        generation: IndexGeneration,
    ) -> Result<Self> {
        let (schema, fields) = build_schema();
        let index = Index::create_in_ram(schema);
        register_lynx_tokenizer(&index);
        let mut writer = index
            .writer(50_000_000)
            .map_err(|e| IndexError::Unavailable(e.to_string()))?;

        for d in docs {
            let doc = to_tantivy_document(&fields, &d);
            writer
                .add_document(doc)
                .map_err(|e| IndexError::Internal(e.to_string()))?;
        }
        writer
            .commit()
            .map_err(|e| IndexError::Internal(e.to_string()))?;
        let reader = index
            .reader()
            .map_err(|e| IndexError::Unavailable(e.to_string()))?;
        let mut generation = generation;
        generation.doc_count = reader.searcher().num_docs();
        Ok(Self {
            reader,
            fields,
            generation,
            stats: std::sync::OnceLock::new(),
        })
    }

    /// Write `docs` as a sealed Tantivy index in `dir`, with its generation manifest.
    ///
    /// The reader is the only writer, and a directory holds exactly one sealed generation
    /// (ADR-0004). The manifest records the tokenizer/ranking versions and checksum, so a
    /// generation can always be attributed to the analysis that built it.
    ///
    /// # Errors
    /// Returns [`IndexError`] if the directory cannot be created, the index cannot be
    /// written and committed, or the manifest cannot be serialised.
    pub fn write_to_dir(
        dir: &Path,
        docs: impl IntoIterator<Item = IndexedDocument>,
        generation: &IndexGeneration,
    ) -> Result<()> {
        std::fs::create_dir_all(dir).map_err(|e| IndexError::Unavailable(e.to_string()))?;
        let (schema, fields) = build_schema();
        let index = Index::create_in_dir(dir, schema)
            .map_err(|e| IndexError::Unavailable(e.to_string()))?;
        register_lynx_tokenizer(&index);
        let mut writer = index
            .writer(50_000_000)
            .map_err(|e| IndexError::Unavailable(e.to_string()))?;

        for d in docs {
            let doc = to_tantivy_document(&fields, &d);
            writer
                .add_document(doc)
                .map_err(|e| IndexError::Internal(e.to_string()))?;
        }
        writer
            .commit()
            .map_err(|e| IndexError::Internal(e.to_string()))?;

        let bytes = serde_json::to_vec_pretty(generation)
            .map_err(|e| IndexError::Internal(e.to_string()))?;
        std::fs::write(dir.join(GENERATION_FILE), bytes)
            .map_err(|e| IndexError::Unavailable(e.to_string()))?;
        Ok(())
    }

    /// Open a generation previously written by [`Self::write_to_dir`].
    ///
    /// # Errors
    /// Returns [`IndexError::Unavailable`] if the directory or manifest is missing or the
    /// index cannot be opened, and [`IndexError::Corrupt`] if the manifest is malformed.
    pub fn open(dir: &Path) -> Result<Self> {
        let manifest_path = dir.join(GENERATION_FILE);
        let bytes = std::fs::read(&manifest_path)
            .map_err(|e| IndexError::Unavailable(format!("{}: {e}", manifest_path.display())))?;
        let mut generation: IndexGeneration = serde_json::from_slice(&bytes)
            .map_err(|e| IndexError::Corrupt(format!("generation manifest: {e}")))?;

        let index = Index::open_in_dir(dir).map_err(|e| IndexError::Unavailable(e.to_string()))?;
        register_lynx_tokenizer(&index);
        let fields = build_fields(&index.schema());
        let reader = index
            .reader()
            .map_err(|e| IndexError::Unavailable(e.to_string()))?;
        generation.doc_count = reader.searcher().num_docs();
        Ok(Self {
            reader,
            fields,
            generation,
            stats: std::sync::OnceLock::new(),
        })
    }

    /// Build the Tantivy query for `plan`, pushing the hard filters that have an exact
    /// tantivy equivalent into the query itself.
    ///
    /// Pushdown coverage:
    ///
    /// - `lang:` — `TermQuery` over the `lang` field, which is indexed lowercased.
    /// - `before:` / `after:` / `freshness:` — `RangeQuery` over the `published_at`
    ///   fast field, with the same day-midnight boundaries `passes_filters` uses. A
    ///   document without a date has no term in the field, so it drops out of the range,
    ///   matching the post-filter.
    ///
    /// `site:` has registrable-domain-suffix semantics in `passes_filters` (it matches
    /// subdomains too) with no exact tantivy equivalent, so it — like `inurl:`, `ext:`
    /// and `filetype:` — stays a post-filter.
    fn build_query(&self, plan: &QueryPlan, now: OffsetDateTime) -> Box<dyn Query> {
        let text = self.fields.text();
        let title = self.fields.title;
        let anchors = self.fields.anchors;
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        let mut positive = false;

        for term in &plan.must {
            if let Some(q) = any_field_query(&text, &tokenize(term.text())) {
                clauses.push((Occur::Must, q));
                positive = true;
            }
        }
        if plan.disjunction {
            for term in &plan.should {
                if let Some(q) = any_field_query(&text, &tokenize(term.text())) {
                    clauses.push((Occur::Should, q));
                    positive = true;
                }
            }
        }
        for phrase in &plan.phrases {
            if let Some(q) = phrase_query(&text, &phrase.text) {
                clauses.push((Occur::Must, q));
                positive = true;
            }
        }
        // `intitle:` is a title-scoped requirement: a `Must` over the title field only, so
        // the term never has to appear in the body. It is a text-field clause, not a
        // metadata filter, so it is deliberately absent from `passes_filters`.
        for value in &plan.filters.intitle {
            if let Some(q) = any_field_query(&[title], &tokenize(value)) {
                clauses.push((Occur::Must, q));
                positive = true;
            }
        }
        // `inanchor:` mirrors `intitle:` over the anchors field.
        for value in &plan.filters.inanchor {
            if let Some(q) = any_field_query(&[anchors], &tokenize(value)) {
                clauses.push((Occur::Must, q));
                positive = true;
            }
        }
        // Negative terms add no clause: they are a scoring penalty, not an exclusion, and
        // the ranker downranks a document that contains one
        // (`docs/search/query-parsing.md` § Filters vs scoring).
        clauses.extend(self.filter_pushdown(plan, now));
        if !positive {
            clauses.insert(0, (Occur::Must, Box::new(AllQuery)));
        }
        Box::new(BooleanQuery::new(clauses))
    }

    /// The pushed-down hard filters as mandatory clauses.
    fn filter_pushdown(
        &self,
        plan: &QueryPlan,
        now: OffsetDateTime,
    ) -> Vec<(Occur, Box<dyn Query>)> {
        let f = &plan.filters;
        let mut pushdown: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        if let Some(lang) = &f.lang {
            let term = Term::from_field_text(self.fields.lang, &lang.to_ascii_lowercase());
            pushdown.push((
                Occur::Must,
                Box::new(TermQuery::new(term, IndexRecordOption::Basic)),
            ));
        }
        // Date boundaries are compared as day boundaries at UTC midnight, the exact
        // contrast `passes_filters` draws on `published_at.date()`.
        let date_field = "published_at".to_string();
        if let Some(before) = f.before {
            pushdown.push((
                Occur::Must,
                Box::new(RangeQuery::new_i64_bounds(
                    date_field.clone(),
                    Bound::Unbounded,
                    Bound::Excluded(before.midnight().assume_utc().unix_timestamp()),
                )),
            ));
        }
        if let Some(after) = f.after {
            pushdown.push((
                Occur::Must,
                Box::new(RangeQuery::new_i64_bounds(
                    date_field.clone(),
                    Bound::Included(after.midnight().assume_utc().unix_timestamp()),
                    Bound::Unbounded,
                )),
            ));
        }
        if let Some(window) = f.freshness {
            let cutoff = now - time::Duration::seconds(window.approx_seconds());
            pushdown.push((
                Occur::Must,
                Box::new(RangeQuery::new_i64_bounds(
                    date_field,
                    Bound::Included(cutoff.unix_timestamp()),
                    Bound::Unbounded,
                )),
            ));
        }
        pushdown
    }
}

fn to_tantivy_document(fields: &Fields, d: &IndexedDocument) -> TantivyDocument {
    let mut doc = TantivyDocument::new();
    doc.add_text(fields.doc_id, d.doc_id.to_string());
    doc.add_text(fields.url, &d.url);
    doc.add_text(fields.title, &d.title);
    doc.add_text(fields.headings, &d.headings);
    doc.add_text(fields.body, &d.body);
    doc.add_text(fields.anchors, &d.anchors);
    doc.add_text(fields.alt, &d.alt);
    doc.add_text(fields.host, d.host.to_ascii_lowercase());
    // `lang` is indexed lowercased so the `lang:` filter pushdown can use an exact term
    // query, consistently with `passes_filters` case-insensitive comparison.
    let lang = d.lang.clone().map(|l| l.to_ascii_lowercase());
    doc.add_text(fields.lang, lang.as_deref().unwrap_or_default());
    doc.add_text(
        fields.content_type,
        d.content_type.clone().unwrap_or_default(),
    );
    add_optional_date(&mut doc, fields.published_at, d.published_at);
    add_optional_date(&mut doc, fields.modified_at, d.modified_at);
    add_optional_date(&mut doc, fields.crawled_at, d.crawled_at);
    doc.add_f64(fields.quality_score, f64::from(d.quality_score));
    doc.add_f64(fields.spam_score, f64::from(d.spam_score));
    doc.add_f64(fields.host_rank, f64::from(d.host_rank));
    doc.add_f64(fields.domain_rank, f64::from(d.domain_rank));
    doc.add_u64(
        fields.content_length,
        u64::from(u32::try_from(d.body.len()).unwrap_or(u32::MAX)),
    );
    doc.add_u64(fields.link_count, u64::from(d.link_count));
    doc.add_u64(fields.cluster_id, d.cluster_id);
    doc.add_text(fields.ranking_config_version, &d.ranking_config_version);
    doc
}

fn add_optional_date(doc: &mut TantivyDocument, field: Field, value: Option<OffsetDateTime>) {
    if let Some(t) = value {
        doc.add_i64(field, t.unix_timestamp());
    }
}

/// The token forms of a plan's positive clauses (`must`, `should`, `intitle:`): what the
/// ranker scores.
fn positive_tokens(plan: &QueryPlan) -> Vec<String> {
    let mut tokens: Vec<String> = plan
        .must
        .iter()
        .chain(&plan.should)
        .flat_map(|t| tokenize(t.text()))
        .collect();
    for value in plan.filters.intitle.iter().chain(&plan.filters.inanchor) {
        tokens.extend(tokenize(value));
    }
    tokens
}

/// Every token a candidate may carry, positive and negative alike. The negative ones are
/// present so the ranker can detect a penalty without a second document lookup; they are
/// filtered back out before BM25F scores the document.
fn query_tokens(plan: &QueryPlan) -> Vec<String> {
    let mut tokens = positive_tokens(plan);
    for term in &plan.must_not {
        tokens.extend(tokenize(term.text()));
    }
    tokens
}

fn any_field_query(text_fields: &[Field], tokens: &[String]) -> Option<Box<dyn Query>> {
    if tokens.is_empty() {
        return None;
    }
    let mut token_clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
    for token in tokens {
        let mut field_clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        for &f in text_fields {
            let term = Term::from_field_text(f, token);
            field_clauses.push((
                Occur::Should,
                Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs)),
            ));
        }
        let field_query: Box<dyn Query> = Box::new(BooleanQuery::new(field_clauses));
        token_clauses.push((Occur::Must, field_query));
    }
    if token_clauses.len() == 1 {
        token_clauses.pop().map(|(_, q)| q)
    } else {
        Some(Box::new(BooleanQuery::new(token_clauses)))
    }
}

fn phrase_query(text_fields: &[Field], text: &str) -> Option<Box<dyn Query>> {
    let tokens = tokenize(text);
    match tokens.len() {
        0 => None,
        1 => any_field_query(text_fields, &tokens),
        _ => {
            let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
            for &f in text_fields {
                let terms: Vec<Term> = tokens.iter().map(|t| Term::from_field_text(f, t)).collect();
                clauses.push((Occur::Should, Box::new(PhraseQuery::new(terms))));
            }
            Some(Box::new(BooleanQuery::new(clauses)))
        }
    }
}

fn text_of(doc: &TantivyDocument, field: Field) -> String {
    doc.get_first(field)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

fn opt_text(doc: &TantivyDocument, field: Field) -> Option<String> {
    let s = text_of(doc, field);
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn opt_i64(doc: &TantivyDocument, field: Field) -> Option<i64> {
    doc.get_first(field).and_then(|v| v.as_i64())
}

#[allow(clippy::cast_possible_truncation)] // scores are f32 by contract
fn f64_of(doc: &TantivyDocument, field: Field) -> f32 {
    doc.get_first(field)
        .and_then(|v| v.as_f64())
        .map_or(0.0, |v| v as f32)
}

fn u64_of(doc: &TantivyDocument, field: Field) -> u64 {
    doc.get_first(field).and_then(|v| v.as_u64()).unwrap_or(0)
}

fn date_of(doc: &TantivyDocument, field: Field) -> Option<OffsetDateTime> {
    opt_i64(doc, field).and_then(|s| OffsetDateTime::from_unix_timestamp(s).ok())
}

/// Frequency maps for a document's stored text fields, tokenised exactly once.
struct FieldTokenCounts {
    title: BTreeMap<String, u32>,
    headings: BTreeMap<String, u32>,
    body: BTreeMap<String, u32>,
    anchors: BTreeMap<String, u32>,
    url: BTreeMap<String, u32>,
    alt: BTreeMap<String, u32>,
}

impl FieldTokenCounts {
    #[allow(clippy::cast_possible_truncation)] // stored fields are far below 2^32 tokens
    fn of(doc: &TantivyDocument, fields: &Fields) -> Self {
        fn counts(text: &str) -> BTreeMap<String, u32> {
            let mut map = BTreeMap::new();
            for token in tokenize(text) {
                let entry = map.entry(token).or_insert(0_u32);
                *entry = entry.saturating_add(1);
            }
            map
        }
        Self {
            title: counts(&text_of(doc, fields.title)),
            headings: counts(&text_of(doc, fields.headings)),
            body: counts(&text_of(doc, fields.body)),
            anchors: counts(&text_of(doc, fields.anchors)),
            url: counts(&text_of(doc, fields.url)),
            alt: counts(&text_of(doc, fields.alt)),
        }
    }

    fn hits(&self, token: &str) -> FieldHits {
        let get = |map: &BTreeMap<String, u32>| map.get(token).copied().unwrap_or(0);
        FieldHits {
            title: get(&self.title),
            headings: get(&self.headings),
            body: get(&self.body),
            anchors: get(&self.anchors),
            url: get(&self.url),
            alt: get(&self.alt),
        }
    }

    /// Each field's token count — its `FieldLengths` value — summed from the maps.
    #[allow(clippy::cast_possible_truncation)] // stored fields are far below 2^32 tokens
    fn lengths(&self) -> FieldLengths {
        let total =
            |map: &BTreeMap<String, u32>| map.values().copied().fold(0_u32, u32::saturating_add);
        FieldLengths {
            title: total(&self.title),
            headings: total(&self.headings),
            body: total(&self.body),
            anchors: total(&self.anchors),
            url: total(&self.url),
            alt: total(&self.alt),
        }
    }
}

/// Weighted term-frequency contribution across a document's stored text, used for
/// field hits and the explanation. This is not Tantivy's BM25; it is a transparent
/// stand-in so field-level provenance is inspectable.
///
/// `counts` must be built once per candidate with [`FieldTokenCounts::of`]; tokenising
/// every field for every query token instead would be quadratic in the field lengths.
#[allow(clippy::cast_precision_loss)] // term frequencies are tiny; f32 precision is ample
fn field_hits_and_terms(
    counts: &FieldTokenCounts,
    tokens: &[String],
    weights: &FieldWeights,
) -> (FieldHits, Vec<TermExplanation>, f32) {
    let mut hits = FieldHits::default();
    let mut terms = Vec::new();
    let mut total = 0.0;

    let sources: [(&str, &BTreeMap<String, u32>, f32); 6] = [
        ("title", &counts.title, weights.title),
        ("headings", &counts.headings, weights.headings),
        ("body", &counts.body, weights.body),
        ("anchors", &counts.anchors, weights.anchors),
        ("url", &counts.url, weights.url),
        ("alt", &counts.alt, weights.alt),
    ];

    let mut seen = BTreeSet::new();
    for token in tokens {
        if !seen.insert(token.as_str()) {
            continue;
        }
        for (name, map, weight) in &sources {
            let Some(&tf) = map.get(token) else { continue };
            if tf == 0 {
                continue;
            }
            let contribution = weight * (tf as f32).min(3.0);
            total += contribution;
            terms.push(TermExplanation {
                term: token.clone(),
                field: (*name).to_string(),
                tf,
                contribution,
            });
            match *name {
                "title" => hits.title += tf,
                "headings" => hits.headings += tf,
                "body" => hits.body += tf,
                "anchors" => hits.anchors += tf,
                "url" => hits.url += tf,
                _ => hits.alt += tf,
            }
        }
    }
    (hits, terms, total)
}

/// Document frequency of `token` across all text fields.
fn document_frequency(searcher: &Searcher, text_fields: &[Field], token: &str) -> u64 {
    let Some(q) = any_field_query(text_fields, std::slice::from_ref(&token.to_string())) else {
        return 0;
    };
    searcher
        .search(q.as_ref(), &Count)
        .map_or(0, |n| u64::try_from(n).unwrap_or(u64::MAX))
}

/// The per-term, per-field frequencies the ranker needs, for the terms present in `doc`.
///
/// `dfs` is the per-search document-frequency cache: DF is a corpus property, so it is
/// looked up once per query token (`search()`) and shared by every candidate, instead of
/// issuing a full-corpus `Count` query per candidate per token.
///
/// `counts` is the per-candidate token-frequency map ([`FieldTokenCounts::of`]), so the
/// stored fields are tokenised once per candidate rather than once per token.
fn term_frequencies_for(
    counts: &FieldTokenCounts,
    tokens: &[String],
    dfs: &mut DocumentFrequencyCache<'_>,
) -> Vec<TermFrequency> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for token in tokens {
        if !seen.insert(token.as_str()) {
            continue;
        }
        let tf = counts.hits(token);
        if tf == FieldHits::default() {
            continue;
        }
        out.push(TermFrequency {
            term: token.clone(),
            df: dfs.get(token),
            tf,
        });
    }
    out
}

/// A per-search cache of document frequencies: query token → corpus DF.
struct DocumentFrequencyCache<'a> {
    searcher: &'a Searcher,
    text_fields: &'a [Field],
    values: BTreeMap<String, u64>,
}

impl DocumentFrequencyCache<'_> {
    /// The corpus DF of `token`, computing it on first use.
    fn get(&mut self, token: &str) -> u64 {
        if let Some(&cached) = self.values.get(token) {
            return cached;
        }
        let df = document_frequency(self.searcher, self.text_fields, token);
        self.values.insert(token.to_string(), df);
        df
    }
}

/// Document frequency of `token` across all text fields.
#[allow(clippy::too_many_arguments)] // one value type per argument keeps the call site flat
fn candidate_of(
    doc: &TantivyDocument,
    fields: &Fields,
    bm25: f32,
    field_hits: FieldHits,
    field_lengths: FieldLengths,
    terms: Vec<TermFrequency>,
) -> Result<Candidate> {
    let doc_id = text_of(doc, fields.doc_id)
        .parse()
        .map_err(|e| IndexError::Corrupt(format!("bad doc_id: {e}")))?;
    Ok(Candidate {
        doc_id,
        url: text_of(doc, fields.url),
        title: text_of(doc, fields.title),
        body: text_of(doc, fields.body),
        lang: opt_text(doc, fields.lang),
        content_type: opt_text(doc, fields.content_type),
        published_at: date_of(doc, fields.published_at),
        modified_at: date_of(doc, fields.modified_at),
        crawled_at: date_of(doc, fields.crawled_at),
        quality_score: f64_of(doc, fields.quality_score),
        spam_score: f64_of(doc, fields.spam_score),
        host_rank: f64_of(doc, fields.host_rank),
        domain_rank: f64_of(doc, fields.domain_rank),
        content_length: u32::try_from(u64_of(doc, fields.content_length)).unwrap_or(u32::MAX),
        link_count: u32::try_from(u64_of(doc, fields.link_count)).unwrap_or(u32::MAX),
        cluster_id: u64_of(doc, fields.cluster_id),
        bm25,
        field_hits,
        field_lengths,
        terms,
    })
}

/// Generous ceiling for a post-filtered scan; the v0.1 reader filters in Rust.
const SCAN_LIMIT: usize = 100_000;

impl SearchIndexReader for TantivyIndexReader {
    fn generation(&self) -> IndexGeneration {
        self.generation.clone()
    }

    fn collection_stats(&self) -> Result<CollectionStats> {
        // Averaged once per reader, never per request: the totals come straight from the
        // segments' inverted indexes (which count exactly the tokens the registered LYNX
        // tokenizer produced), so no stored field is re-tokenised or fetched.
        if let Some(cached) = self.stats.get() {
            return Ok(*cached);
        }
        let searcher = self.reader.searcher();
        let text_fields = self.fields.text();
        let mut totals = [0_u64; 6];
        for segment in searcher.segment_readers() {
            for (slot, &field) in text_fields.iter().enumerate() {
                let index = segment
                    .inverted_index(field)
                    .map_err(|e| IndexError::Corrupt(e.to_string()))?;
                totals[slot] = totals[slot].saturating_add(index.total_num_tokens());
            }
        }
        // `Fields::text` is [title, headings, anchors, url, alt, body]; the averages
        // struct is (title, headings, body, anchors, url, alt).
        let [title, headings, anchors, url, alt, body] = totals;
        let doc_count = searcher.num_docs();
        let stats = if doc_count == 0 {
            CollectionStats::default()
        } else {
            #[allow(clippy::cast_precision_loss)] // token counts fit f32 exactly
            #[allow(clippy::cast_possible_truncation)] // an average field length barely spans f32's range
            fn average(total_tokens: u64, doc_count: u64) -> f32 {
                ((total_tokens as f64) / f64::from(u32::try_from(doc_count).unwrap_or(u32::MAX)))
                    as f32
            }
            CollectionStats {
                doc_count,
                avg: crate::reader::AvgFieldLengths {
                    title: average(title, doc_count),
                    headings: average(headings, doc_count),
                    body: average(body, doc_count),
                    anchors: average(anchors, doc_count),
                    url: average(url, doc_count),
                    alt: average(alt, doc_count),
                },
            }
        };
        let _ = self.stats.set(stats);
        Ok(stats)
    }

    fn search(&self, plan: &QueryPlan, opts: &SearchOptions) -> Result<Vec<Candidate>> {
        let searcher = self.reader.searcher();
        let query = self.build_query(plan, opts.now);
        let wanted = opts.limit.saturating_add(opts.offset).min(SCAN_LIMIT);
        let top = searcher
            .search(query.as_ref(), &TopDocs::with_limit(wanted.max(1)))
            .map_err(|e| IndexError::Internal(e.to_string()))?;

        let tokens = query_tokens(plan);
        let text_fields = self.fields.text();
        let mut dfs = DocumentFrequencyCache {
            searcher: &searcher,
            text_fields: &text_fields,
            values: BTreeMap::new(),
        };

        let mut out = Vec::new();
        for (score, address) in top {
            let doc: TantivyDocument = searcher
                .doc(address)
                .map_err(|e| IndexError::Corrupt(e.to_string()))?;
            let d_url = text_of(&doc, self.fields.url);
            let host = text_of(&doc, self.fields.host);
            let lang = opt_text(&doc, self.fields.lang);
            let ctype = opt_text(&doc, self.fields.content_type);
            let published = date_of(&doc, self.fields.published_at);
            if !passes_filters(
                plan,
                &d_url,
                &host,
                lang.as_deref(),
                ctype.as_deref(),
                published,
                opts.now,
            ) {
                continue;
            }
            let counts = FieldTokenCounts::of(&doc, &self.fields);
            // Field lengths are the token counts the same pass just produced.
            let field_lengths = counts.lengths();
            let (hits, _terms, _standin) = field_hits_and_terms(&counts, &tokens, &opts.weights);
            let terms = term_frequencies_for(&counts, &tokens, &mut dfs);
            out.push(candidate_of(
                &doc,
                &self.fields,
                score,
                hits,
                field_lengths,
                terms,
            )?);
        }

        // Tantivy's `TopDocs` tie order is unspecified; the reference reader ties on
        // doc-id ascending, so re-sort to the same order for deterministic pagination
        // and backend parity.
        out.sort_by(|a, b| {
            b.bm25
                .total_cmp(&a.bm25)
                .then_with(|| a.doc_id.cmp(&b.doc_id))
        });
        Ok(out.into_iter().skip(opts.offset).take(opts.limit).collect())
    }

    fn count(&self, plan: &QueryPlan, now: OffsetDateTime) -> Result<u64> {
        let searcher = self.reader.searcher();
        let query = self.build_query(plan, now);
        // The pushdown is exact for language and date filters only; `site:`, `inurl:`,
        // `ext:` and `filetype:` are post-filters, so a plain `Count` would over-count.
        let f = &plan.filters;
        if f.site.is_none() && f.inurl.is_none() && f.ext.is_empty() && f.filetype.is_empty() {
            return searcher
                .search(query.as_ref(), &Count)
                .map(|n| u64::try_from(n).unwrap_or(u64::MAX))
                .map_err(|e| IndexError::Internal(e.to_string()));
        }
        // Otherwise count through the same post-filter a search applies, bounded by
        // [`SCAN_LIMIT`].
        let top = searcher
            .search(query.as_ref(), &TopDocs::with_limit(SCAN_LIMIT))
            .map_err(|e| IndexError::Internal(e.to_string()))?;
        let mut survivors = 0_u64;
        for (_, address) in top {
            let doc: TantivyDocument = searcher
                .doc(address)
                .map_err(|e| IndexError::Corrupt(e.to_string()))?;
            if passes_filters(
                plan,
                &text_of(&doc, self.fields.url),
                &text_of(&doc, self.fields.host),
                opt_text(&doc, self.fields.lang).as_deref(),
                opt_text(&doc, self.fields.content_type).as_deref(),
                date_of(&doc, self.fields.published_at),
                now,
            ) {
                survivors = survivors.saturating_add(1);
            }
        }
        Ok(survivors)
    }

    fn document(&self, doc_id: DocId) -> Result<Option<DocumentMetadata>> {
        let searcher = self.reader.searcher();
        let term = Term::from_field_text(self.fields.doc_id, &doc_id.to_string());
        let query = TermQuery::new(term, IndexRecordOption::Basic);
        let top = searcher
            .search(&query, &TopDocs::with_limit(1))
            .map_err(|e| IndexError::Internal(e.to_string()))?;
        let Some((_, address)) = top.into_iter().next() else {
            return Ok(None);
        };
        let doc: TantivyDocument = searcher
            .doc(address)
            .map_err(|e| IndexError::Corrupt(e.to_string()))?;
        Ok(Some(DocumentMetadata {
            doc_id,
            url: text_of(&doc, self.fields.url),
            title: text_of(&doc, self.fields.title),
            lang: opt_text(&doc, self.fields.lang),
            content_type: opt_text(&doc, self.fields.content_type),
            published_at: date_of(&doc, self.fields.published_at),
            crawled_at: date_of(&doc, self.fields.crawled_at),
            content_length: u32::try_from(u64_of(&doc, self.fields.content_length))
                .unwrap_or(u32::MAX),
            cluster_id: u64_of(&doc, self.fields.cluster_id),
            body: text_of(&doc, self.fields.body),
            ranking_config_version: text_of(&doc, self.fields.ranking_config_version),
        }))
    }

    fn explain(&self, plan: &QueryPlan, doc_id: DocId) -> Result<Explanation> {
        let searcher = self.reader.searcher();
        let term = Term::from_field_text(self.fields.doc_id, &doc_id.to_string());
        let query = TermQuery::new(term, IndexRecordOption::Basic);
        let top = searcher
            .search(&query, &TopDocs::with_limit(1))
            .map_err(|e| IndexError::Internal(e.to_string()))?;
        let Some((_, address)) = top.into_iter().next() else {
            return Ok(Explanation {
                doc_id,
                bm25: 0.0,
                terms: Vec::new(),
            });
        };
        let doc: TantivyDocument = searcher
            .doc(address)
            .map_err(|e| IndexError::Corrupt(e.to_string()))?;
        let tokens = positive_tokens(plan);
        let counts = FieldTokenCounts::of(&doc, &self.fields);
        let (_hits, terms, total) =
            field_hits_and_terms(&counts, &tokens, &FieldWeights::default());
        Ok(Explanation {
            doc_id,
            bm25: total,
            terms,
        })
    }
}
