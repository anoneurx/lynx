//! The parse → store → index pipeline.
//!
//! The indexer sits at the end of the crawl plane: it receives a [`ParsedDocument`] from
//! the parser, records the document version and its outbound edges in the operational
//! store, and offers the document to a [`GenerationBuilder`]. The store is a trait so the
//! pipeline can run against an in-memory store in tests and against Postgres in
//! production without changing this code (`docs/architecture/specification.md` § E —
//! `services/indexer` writes the index and the document plane).
//!
//! Writing to the live index and publishing its generation is a separate, later step; the
//! pipeline only accumulates a generation in memory.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::RwLock;

use lynx_common::ids::DocumentId;
use lynx_models::domain::ParsedDocument;
use thiserror::Error;
use time::OffsetDateTime;

use crate::build::{AddOutcome, GenerationBuilder};
use crate::memory::IndexedDocument;
use crate::reader::IndexGeneration;

/// A boxed, `Send` future, so the store trait stays object-safe.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A document version as offered to the store, before the store assigns its number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRecord {
    /// Document identity, derived from the canonical URL.
    pub doc_id: DocumentId,
    /// Canonical URL.
    pub canonical: String,
    /// Content hash of the fetched body.
    pub content_hash: String,
    /// HTTP status, if the document came from a fetch.
    pub status_code: Option<u16>,
    /// The extracted text, retained for re-indexing and takedown.
    pub extracted_text: Option<String>,
}

/// A persisted document version, after the store assigned its monotonic number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentVersion {
    /// Document identity.
    pub doc_id: DocumentId,
    /// Canonical URL.
    pub canonical: String,
    /// Content hash of the fetched body.
    pub content_hash: String,
    /// HTTP status, if any.
    pub status_code: Option<u16>,
    /// The extracted text.
    pub extracted_text: Option<String>,
    /// Monotonic version number, starting at 1 per document.
    pub version: u32,
}

/// A link-graph edge from one document to a target URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkEdge {
    /// Source document.
    pub source_doc_id: DocumentId,
    /// Target canonical URL.
    pub target_canonical: String,
    /// Anchor text, when the edge came from an anchor element.
    pub anchor: Option<String>,
}

/// A store failure. Deliberately opaque: the store owns its own error detail.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StoreError {
    /// The store could not complete the write.
    #[error("document store failed: {0}")]
    Failed(String),
}

/// The operational-store seam the pipeline writes through.
///
/// Methods are boxed-future async so the trait is object-safe and a Postgres store can be
/// swapped in without touching the pipeline.
pub trait DocumentStore: Send + Sync {
    /// Record a new version of a document, returning the assigned version number.
    fn record_version(&self, record: VersionRecord) -> BoxFuture<'_, Result<u32, StoreError>>;

    /// Record the outbound edges of a document.
    fn record_links(&self, edges: Vec<LinkEdge>) -> BoxFuture<'_, Result<(), StoreError>>;
}

/// What the pipeline did with one document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestOutcome {
    /// The document identity.
    pub doc_id: DocumentId,
    /// The version number the store assigned.
    pub version: u32,
    /// What the generation builder did with the document.
    pub outcome: AddOutcome,
}

/// Drives parsed documents into the store and a generation.
#[derive(Debug)]
pub struct IndexPipeline<S: DocumentStore> {
    store: S,
    builder: GenerationBuilder,
}

impl<S: DocumentStore> IndexPipeline<S> {
    /// A pipeline that seals its generation at `created_at`.
    #[must_use]
    pub fn new(store: S, created_at: OffsetDateTime) -> Self {
        Self {
            store,
            builder: GenerationBuilder::new(created_at),
        }
    }

    /// A pipeline that adds to an existing builder.
    #[must_use]
    pub fn with_builder(store: S, builder: GenerationBuilder) -> Self {
        Self { store, builder }
    }

    /// Record a document version and its edges, then offer it to the generation.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if the store fails. The document is not added to the
    /// generation in that case, so the index never claims a version the store did not
    /// record.
    pub async fn ingest(
        &mut self,
        parsed: ParsedDocument,
        status_code: Option<u16>,
    ) -> Result<IngestOutcome, StoreError> {
        let doc_id = DocumentId::from_canonical_url(&parsed.canonical);

        let version = self
            .store
            .record_version(VersionRecord {
                doc_id,
                canonical: parsed.canonical.clone(),
                content_hash: parsed.content_hash.clone(),
                status_code,
                extracted_text: Some(parsed.body.clone()),
            })
            .await?;

        if !parsed.links.is_empty() {
            let edges = parsed
                .links
                .iter()
                .map(|target| LinkEdge {
                    source_doc_id: doc_id,
                    target_canonical: target.clone(),
                    anchor: None,
                })
                .collect();
            self.store.record_links(edges).await?;
        }

        let outcome = self.builder.add(parsed);
        Ok(IngestOutcome {
            doc_id,
            version,
            outcome,
        })
    }

    /// Borrow the store.
    #[must_use]
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Borrow the generation builder.
    #[must_use]
    pub fn builder(&self) -> &GenerationBuilder {
        &self.builder
    }

    /// Seal the accumulated generation, returning its documents and manifest.
    #[must_use]
    pub fn seal(self, generation_id: u64) -> (Vec<IndexedDocument>, IndexGeneration) {
        self.builder.build(generation_id)
    }
}

/// An in-memory [`DocumentStore`] for tests and single-process runs.
///
/// It mirrors the shape of the Postgres write path — monotonic versions per document and
/// an edge set — so a hermetic pipeline test exercises the same contract.
#[derive(Debug, Default)]
pub struct MemoryDocumentStore {
    versions: RwLock<Vec<DocumentVersion>>,
    links: RwLock<Vec<LinkEdge>>,
    counters: RwLock<HashMap<DocumentId, u32>>,
}

impl MemoryDocumentStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every version recorded, in insertion order.
    #[must_use]
    pub fn versions(&self) -> Vec<DocumentVersion> {
        read(&self.versions)
    }

    /// Every edge recorded, in insertion order.
    #[must_use]
    pub fn links(&self) -> Vec<LinkEdge> {
        read(&self.links)
    }

    /// Versions recorded for one document.
    #[must_use]
    pub fn versions_of(&self, doc_id: DocumentId) -> Vec<DocumentVersion> {
        self.versions()
            .into_iter()
            .filter(|v| v.doc_id == doc_id)
            .collect()
    }
}

/// A read-only snapshot. A poisoned lock yields the empty value rather than panicking in
/// a test accessor.
fn read<T>(lock: &RwLock<T>) -> T
where
    T: Clone + Default,
{
    lock.read().map(|guard| guard.clone()).unwrap_or_default()
}

impl DocumentStore for MemoryDocumentStore {
    fn record_version(&self, record: VersionRecord) -> BoxFuture<'_, Result<u32, StoreError>> {
        Box::pin(async move {
            let mut counters = self
                .counters
                .write()
                .map_err(|_| StoreError::Failed("counter lock poisoned".into()))?;
            let version = counters.entry(record.doc_id).or_insert(0);
            *version += 1;
            let assigned = *version;
            drop(counters);

            self.versions
                .write()
                .map_err(|_| StoreError::Failed("version lock poisoned".into()))?
                .push(DocumentVersion {
                    doc_id: record.doc_id,
                    canonical: record.canonical,
                    content_hash: record.content_hash,
                    status_code: record.status_code,
                    extracted_text: record.extracted_text,
                    version: assigned,
                });
            Ok(assigned)
        })
    }

    fn record_links(&self, edges: Vec<LinkEdge>) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            self.links
                .write()
                .map_err(|_| StoreError::Failed("link lock poisoned".into()))?
                .extend(edges);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn parsed(url: &str, body: &str) -> ParsedDocument {
        ParsedDocument {
            canonical: url.to_string(),
            title: "T".to_string(),
            body: body.to_string(),
            content_hash: format!("h-{url}-{body}"),
            links: vec!["https://e.com/out".to_string()],
            ..ParsedDocument::default()
        }
    }

    #[tokio::test]
    async fn ingest_records_a_version_and_links_then_indexes() {
        let mut pipeline =
            IndexPipeline::new(MemoryDocumentStore::new(), datetime!(2026-01-01 00:00 UTC));
        let outcome = pipeline
            .ingest(parsed("https://e.com/a", "hello world"), Some(200))
            .await
            .unwrap();
        assert_eq!(outcome.outcome, AddOutcome::Indexed);
        assert_eq!(outcome.version, 1);

        let store = pipeline.store();
        assert_eq!(store.versions().len(), 1);
        assert_eq!(store.links().len(), 1);
        assert_eq!(store.links()[0].target_canonical, "https://e.com/out");

        let (docs, generation) = pipeline.seal(7);
        assert_eq!(docs.len(), 1);
        assert_eq!(generation.id, 7);
    }

    #[tokio::test]
    async fn versions_increment_and_reingest_is_idempotent_in_the_index() {
        let mut pipeline =
            IndexPipeline::new(MemoryDocumentStore::new(), datetime!(2026-01-01 00:00 UTC));
        pipeline
            .ingest(parsed("https://e.com/a", "one"), Some(200))
            .await
            .unwrap();
        let second = pipeline
            .ingest(parsed("https://e.com/a", "one"), Some(200))
            .await
            .unwrap();
        assert_eq!(second.version, 2);
        assert_eq!(second.outcome, AddOutcome::Unchanged);
        let store = pipeline.store();
        assert_eq!(store.versions_of(second.doc_id).len(), 2);
    }

    #[tokio::test]
    async fn noindex_documents_are_recorded_but_not_indexed() {
        let mut pipeline =
            IndexPipeline::new(MemoryDocumentStore::new(), datetime!(2026-01-01 00:00 UTC));
        let mut doc = parsed("https://e.com/secret", "hidden");
        doc.noindex = true;
        let outcome = pipeline.ingest(doc, Some(200)).await.unwrap();
        assert_eq!(outcome.outcome, AddOutcome::Rejected("noindex"));
        let store = pipeline.store();
        assert_eq!(store.versions().len(), 1);
        let (docs, _) = pipeline.seal(1);
        assert!(docs.is_empty());
    }
}
