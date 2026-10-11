//! `lynx-indexer` — index generations and the readable index abstraction.
//!
//! This crate owns:
//!
//! - [`reader`] — the [`SearchIndexReader`](reader::SearchIndexReader) trait and its value
//!   types. This is the boundary that keeps the index engine replaceable; it pulls in no
//!   engine dependency itself.
//! - [`memory`] — an in-memory reference reader used by tests.
//! - [`build`] — [`GenerationBuilder`](build::GenerationBuilder), the engine-agnostic
//!   indexer that seals parsed documents into an immutable generation.
//!
//! A Tantivy-backed reader *and writer* are available behind the `tantivy` feature.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

pub mod build;
pub mod memory;
pub mod reader;
pub mod tokenizer;

#[cfg(feature = "tantivy")]
pub mod tantivy_reader;

#[cfg(feature = "tantivy")]
pub use tantivy_reader::TantivyIndexReader;

pub use build::{AddOutcome, GenerationBuilder, RANKING_CONFIG_VERSION, TOKENIZER_VERSION};
pub use memory::{InMemoryIndex, IndexedDocument};
pub use reader::{
    passes_filters, AvgFieldLengths, Candidate, CollectionStats, DocId, DocumentMetadata,
    Explanation, FieldHits, FieldLengths, FieldWeights, IndexError, IndexGeneration,
    SearchIndexReader, SearchOptions, TermExplanation, TermFrequency,
};

/// The service name, used in logs and metrics.
pub const SERVICE_NAME: &str = "indexer";

#[cfg(test)]
mod tests {
    #[test]
    fn service_name_is_stable() {
        assert_eq!(super::SERVICE_NAME, "indexer");
    }
}
