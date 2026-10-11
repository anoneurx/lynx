//! A Tantivy-backed [`SearchIndexReader`], available behind the `tantivy` feature.
//!
//! Tantivy types are confined to this module. The rest of the workspace sees only the
//! trait and its value types, so the engine stays replaceable (see ADR-0002). This is the
//! production reader; the in-memory [`crate::memory::InMemoryIndex`] is the reference.

use std::path::Path;

use lynx_query::QueryPlan;
use tantivy::collector::TopDocs;
use tantivy::query::{AllQuery, BooleanQuery, Occur, PhraseQuery, Query, TermQuery};
use tantivy::schema::document::Value as _;
use tantivy::schema::{
    Field, IndexRecordOption, Schema, TantivyDocument, Term, STORED, STRING, TEXT,
};
use tantivy::{Index, IndexReader};
use time::OffsetDateTime;

use crate::memory::IndexedDocument;
use crate::reader::{
    passes_filters, Candidate, DocId, DocumentMetadata, Explanation, FieldHits, FieldWeights,
    IndexError, IndexGeneration, Result, SearchIndexReader, SearchOptions, TermExplanation,
};
use crate::tokenizer::tokenize;

/// The manifest file written alongside a generation's index segments.
const GENERATION_FILE: &str = "generation.json";

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
    b.add_text_field("url", TEXT | STORED);
    b.add_text_field("title", TEXT | STORED);
    b.add_text_field("headings", TEXT | STORED);
    b.add_text_field("body", TEXT | STORED);
    b.add_text_field("anchors", TEXT | STORED);
    b.add_text_field("alt", TEXT | STORED);
    b.add_text_field("host", STRING | STORED);
    b.add_text_field("lang", STRING | STORED);
    b.add_text_field("content_type", STRING | STORED);
    b.add_i64_field("published_at", STORED);
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
        let fields = build_fields(&index.schema());
        let reader = index
            .reader()
            .map_err(|e| IndexError::Unavailable(e.to_string()))?;
        generation.doc_count = reader.searcher().num_docs();
        Ok(Self {
            reader,
            fields,
            generation,
        })
    }

    fn build_query(&self, plan: &QueryPlan) -> Box<dyn Query> {
        let text = self.fields.text();
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
        for term in &plan.must_not {
            if let Some(q) = any_field_query(&text, &tokenize(term.text())) {
                clauses.push((Occur::MustNot, q));
            }
        }
        if !positive {
            clauses.insert(0, (Occur::Must, Box::new(AllQuery)));
        }
        Box::new(BooleanQuery::new(clauses))
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
    doc.add_text(fields.host, &d.host);
    doc.add_text(fields.lang, d.lang.clone().unwrap_or_default());
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

/// Weighted term-frequency contribution across a document's stored text, used for
/// field hits and the explanation. This is not Tantivy's BM25; it is a transparent
/// stand-in so field-level provenance is inspectable.
#[allow(clippy::cast_precision_loss)] // term frequencies are tiny; f32 precision is ample
fn field_hits_and_terms(
    doc: &TantivyDocument,
    fields: &Fields,
    tokens: &[String],
    weights: &FieldWeights,
) -> (FieldHits, Vec<TermExplanation>, f32) {
    let mut hits = FieldHits::default();
    let mut terms = Vec::new();
    let mut total = 0.0;

    let sources: [(&str, Field, f32); 6] = [
        ("title", fields.title, weights.title),
        ("headings", fields.headings, weights.headings),
        ("body", fields.body, weights.body),
        ("anchors", fields.anchors, weights.anchors),
        ("url", fields.url, weights.url),
        ("alt", fields.alt, weights.alt),
    ];

    for token in tokens {
        for (name, field, weight) in sources {
            let text = text_of(doc, field);
            let tf = u32::try_from(tokenize(&text).iter().filter(|t| *t == token).count())
                .unwrap_or(u32::MAX);
            if tf == 0 {
                continue;
            }
            let contribution = weight * (tf as f32).min(3.0);
            total += contribution;
            terms.push(TermExplanation {
                term: token.clone(),
                field: name.to_string(),
                tf,
                contribution,
            });
            match name {
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

fn candidate_of(
    doc: &TantivyDocument,
    fields: &Fields,
    bm25: f32,
    field_hits: FieldHits,
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
    })
}

/// Generous ceiling for a post-filtered scan; the v0.1 reader filters in Rust.
const SCAN_LIMIT: usize = 100_000;

impl SearchIndexReader for TantivyIndexReader {
    fn generation(&self) -> IndexGeneration {
        self.generation.clone()
    }

    fn search(&self, plan: &QueryPlan, opts: &SearchOptions) -> Result<Vec<Candidate>> {
        let searcher = self.reader.searcher();
        let query = self.build_query(plan);
        let wanted = opts.limit.saturating_add(opts.offset).min(SCAN_LIMIT);
        let top = searcher
            .search(query.as_ref(), &TopDocs::with_limit(wanted.max(1)))
            .map_err(|e| IndexError::Internal(e.to_string()))?;

        let tokens: Vec<String> = plan
            .must
            .iter()
            .chain(&plan.should)
            .flat_map(|t| tokenize(t.text()))
            .collect();

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
            let (hits, _terms, _standin) =
                field_hits_and_terms(&doc, &self.fields, &tokens, &opts.weights);
            out.push(candidate_of(&doc, &self.fields, score, hits)?);
        }

        Ok(out.into_iter().skip(opts.offset).take(opts.limit).collect())
    }

    fn count(&self, plan: &QueryPlan) -> Result<u64> {
        let opts = SearchOptions {
            limit: SCAN_LIMIT,
            offset: 0,
            ..SearchOptions::default()
        };
        Ok(u64::try_from(self.search(plan, &opts)?.len()).unwrap_or(u64::MAX))
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
        let tokens: Vec<String> = plan
            .must
            .iter()
            .chain(&plan.should)
            .flat_map(|t| tokenize(t.text()))
            .collect();
        let (_hits, terms, total) =
            field_hits_and_terms(&doc, &self.fields, &tokens, &FieldWeights::default());
        Ok(Explanation {
            doc_id,
            bm25: total,
            terms,
        })
    }
}
