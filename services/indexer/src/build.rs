//! Building an index generation from parsed documents.
//!
//! This is the minimal indexer (Step 4): it maps a [`ParsedDocument`] to an
//! [`IndexedDocument`], normalises its text, rejects documents with nothing to index,
//! skips unchanged content for idempotency, and seals the result into an
//! [`IndexGeneration`] with a deterministic checksum.
//!
//! The builder is engine-agnostic. It pulls in no Tantivy type; the Tantivy writer
//! (`tantivy_reader::write_to_dir`, feature `tantivy`) consumes its output. This keeps the
//! generation-assembly logic testable without the engine, exactly like the reader.
//!
//! Scope note: v0.1 does the first stages only — normalisation, field assembly, empty-text
//! rejection, idempotency, and generation sealing. Language confirmation, stopwords,
//! stemming, dedup, and link-graph extraction land in later steps; the interface here is
//! shaped so they can be added without changing callers. See `docs/indexing/pipeline.md`.

use std::collections::BTreeMap;

use lynx_common::hash::ContentHash;
use lynx_common::ids::DocumentId;
use lynx_common::text;
use lynx_common::time::Clock;
use lynx_models::domain::ParsedDocument;
use time::OffsetDateTime;

use crate::memory::IndexedDocument;
use crate::reader::IndexGeneration;

/// Analyzer/tokenizer version. Any change to tokenisation bumps this and forces a rebuild
/// (see `docs/indexing/pipeline.md` § Stage 5).
pub const TOKENIZER_VERSION: &str = "lynx-tokenizer-1";

/// Ranking config version. Tuning weights does not require a rebuild; this is recorded for
/// attribution only (`docs/indexing/index-schema.md`).
pub const RANKING_CONFIG_VERSION: &str = "v0.1.0";

/// The number of shards a v0.1 generation is written with.
pub const SHARDS: u32 = 1;

/// What happened to a document offered to [`GenerationBuilder::add`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddOutcome {
    /// The document was added to the generation.
    Indexed,
    /// The document's content hash matched the version already held; the write was skipped.
    Unchanged,
    /// The document had no indexable text; it was rejected. The string is the reason.
    Rejected(&'static str),
}

/// Accumulates parsed documents into a single immutable generation.
///
/// Documents are keyed by the [`DocumentId`] derived from their canonical URL, so
/// re-adding a document replaces it, and re-adding identical content is a no-op.
#[derive(Debug)]
pub struct GenerationBuilder {
    docs: Vec<IndexedDocument>,
    /// Content hash of the version currently held for each document, for idempotency.
    hashes: BTreeMap<DocumentId, String>,
    /// Position of each held document in `docs`, so an update is O(1) instead of a
    /// linear `find` that makes a full re-index O(n²).
    positions: std::collections::HashMap<DocumentId, usize>,
    tokenizer_version: String,
    ranking_config_version: String,
    created_at: OffsetDateTime,
    shards: u32,
}

impl GenerationBuilder {
    /// A builder that will seal its generation at `created_at`.
    #[must_use]
    pub fn new(created_at: OffsetDateTime) -> Self {
        Self {
            docs: Vec::new(),
            hashes: BTreeMap::new(),
            positions: std::collections::HashMap::new(),
            tokenizer_version: TOKENIZER_VERSION.to_string(),
            ranking_config_version: RANKING_CONFIG_VERSION.to_string(),
            created_at,
            shards: SHARDS,
        }
    }

    /// A builder sealed at the current time of `clock`.
    #[must_use]
    pub fn now(clock: &dyn Clock) -> Self {
        Self::new(clock.now())
    }

    /// Override the tokenizer version (used when an analyzer changes).
    #[must_use]
    pub fn with_tokenizer_version(mut self, version: impl Into<String>) -> Self {
        self.tokenizer_version = version.into();
        self
    }

    /// Override the ranking config version.
    #[must_use]
    pub fn with_ranking_config_version(mut self, version: impl Into<String>) -> Self {
        self.ranking_config_version = version.into();
        self
    }

    /// Override the shard count. Zero is clamped to one: a generation has at least one shard.
    #[must_use]
    pub fn with_shards(mut self, shards: u32) -> Self {
        self.shards = shards.max(1);
        self
    }

    /// Offer a parsed document to the builder.
    ///
    /// Returns [`AddOutcome::Unchanged`] when the content hash matches the version already
    /// held — the write is skipped so a full re-index does not churn segments. Returns
    /// [`AddOutcome::Rejected`] when the document has no indexable text; any previously
    /// indexed version of the same URL is removed so the index reflects the new state.
    pub fn add(&mut self, parsed: ParsedDocument) -> AddOutcome {
        let doc_id = DocumentId::from_canonical_url(&parsed.canonical);

        if parsed.noindex {
            // The document asked not to be indexed. Honor it and drop any previously
            // indexed version so the live index matches the site's current instruction.
            self.remove(&doc_id);
            return AddOutcome::Rejected("noindex");
        }

        if self.hashes.get(&doc_id) == Some(&parsed.content_hash) {
            return AddOutcome::Unchanged;
        }

        let Some(indexed) = to_indexed_document(doc_id, &parsed) else {
            // The new version has nothing to index. Drop any stale entry so the live
            // index matches the source of truth (pipeline.md § Failure behaviour).
            self.remove(&doc_id);
            return AddOutcome::Rejected("empty_text");
        };

        if let Some(&slot) = self.positions.get(&doc_id) {
            self.docs[slot] = indexed;
        } else {
            self.positions.insert(doc_id, self.docs.len());
            self.docs.push(indexed);
        }
        self.hashes.insert(doc_id, parsed.content_hash);
        AddOutcome::Indexed
    }

    /// Drop a document (and its position entry), keeping the position map consistent.
    ///
    /// `swap_remove` reorders `docs`, so the displaced document's position is updated;
    /// `build()` sorts by id anyway, so iteration order is not observable.
    fn remove(&mut self, doc_id: &DocumentId) {
        self.hashes.remove(doc_id);
        let Some(slot) = self.positions.remove(doc_id) else {
            return;
        };
        let last = self.docs.len() - 1;
        let moved = self.docs[last].doc_id;
        self.docs.swap_remove(slot);
        // When the removed document was not the last, the displaced one takes its slot.
        if slot < last {
            self.positions.insert(moved, slot);
        }
    }

    /// Number of documents currently held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.docs.len()
    }

    /// Whether no documents are held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// Seal the generation, returning its documents (sorted by id for determinism) and its
    /// manifest.
    #[must_use]
    pub fn build(mut self, id: u64) -> (Vec<IndexedDocument>, IndexGeneration) {
        self.docs.sort_by_key(|a| a.doc_id);
        let size_bytes = self.docs.iter().map(estimate_size).sum();
        let checksum = self.checksum();
        let generation = IndexGeneration {
            id,
            tokenizer_version: self.tokenizer_version,
            ranking_config_version: self.ranking_config_version,
            created_at: self.created_at,
            doc_count: u64::try_from(self.docs.len()).unwrap_or(u64::MAX),
            size_bytes,
            checksum,
            shards: self.shards,
        };
        (self.docs, generation)
    }

    /// A deterministic checksum over the sorted `(doc_id, content_hash)` pairs. Two builders
    /// that saw the same documents in any order produce the same checksum.
    fn checksum(&self) -> String {
        let mut input = String::new();
        for (id, hash) in &self.hashes {
            input.push_str(&id.to_string());
            input.push('\u{1f}');
            input.push_str(hash);
            input.push('\n');
        }
        ContentHash::of(input.as_bytes()).as_str().to_string()
    }
}

/// Map a parsed document into an indexed one, or `None` when it has no indexable text.
///
/// Text is cleaned (invisible/control characters stripped, whitespace collapsed) and
/// compatibility-normalised. Display case is preserved: case folding is the analyzer's job
/// at index time, not ours, so snippets read as the page was written.
fn to_indexed_document(doc_id: DocumentId, parsed: &ParsedDocument) -> Option<IndexedDocument> {
    let title = display(&parsed.title);
    let headings = display(&parsed.headings.join(" "));
    let body = display(&parsed.body);

    if title.is_empty() && headings.is_empty() && body.is_empty() {
        return None;
    }

    let anchors = display(&parsed.anchors.join(" "));
    let host = host_of(&parsed.canonical);
    let lang = parsed
        .lang
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase);

    Some(IndexedDocument {
        doc_id,
        url: parsed.canonical.clone(),
        host,
        title,
        headings,
        body,
        anchors,
        alt: display(&parsed.alt),
        lang,
        content_type: None,
        published_at: parsed.published_at,
        modified_at: parsed.modified_at,
        crawled_at: None,
        quality_score: 0.5,
        spam_score: 0.0,
        host_rank: 0.0,
        domain_rank: 0.0,
        link_count: u32::try_from(parsed.links.len()).unwrap_or(u32::MAX),
        cluster_id: parsed.simhash,
        ranking_config_version: RANKING_CONFIG_VERSION.to_string(),
    })
}

/// Clean and compatibility-normalise display text.
fn display(input: &str) -> String {
    text::nfkc(&text::clean(input))
}

/// Extract the host from a canonical URL, best-effort.
fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(ToString::to_string))
        .unwrap_or_default()
}

/// A rough on-disk size estimate: the stored text length. Good enough to report a
/// generation's growth; the Tantivy writer records the true size when it lands.
fn estimate_size(doc: &IndexedDocument) -> u64 {
    let text_bytes = doc.url.len()
        + doc.title.len()
        + doc.headings.len()
        + doc.body.len()
        + doc.anchors.len()
        + doc.alt.len();
    u64::try_from(text_bytes).unwrap_or(u64::MAX)
}

#[cfg(feature = "tantivy")]
impl GenerationBuilder {
    /// Seal the generation and write it to `dir` as a Tantivy index, returning its manifest.
    ///
    /// Combines [`Self::build`] with
    /// [`TantivyIndexReader::write_to_dir`](crate::tantivy_reader::TantivyIndexReader::write_to_dir).
    ///
    /// # Errors
    /// Returns [`IndexError`](crate::reader::IndexError) if the index cannot be written.
    pub fn write_to_dir(
        self,
        dir: &std::path::Path,
        id: u64,
    ) -> crate::reader::Result<IndexGeneration> {
        let (docs, generation) = self.build(id);
        crate::tantivy_reader::TantivyIndexReader::write_to_dir(dir, docs, &generation)?;
        Ok(generation)
    }
}
