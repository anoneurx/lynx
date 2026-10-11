//! `lynx-indexer` — index generations and the readable index abstraction.
//!
//! This crate owns two things:
//!
//! - [`reader`] — the [`SearchIndexReader`](reader::SearchIndexReader) trait and its value
//!   types. This is the boundary that keeps the index engine replaceable; it pulls in no
//!   engine dependency itself.
//! - [`memory`] — an in-memory reference reader used by tests.
//!
//! A Tantivy-backed reader is available behind the `tantivy` feature. The indexer that
//! *writes* generations lands in Step 4.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

pub mod memory;
pub mod reader;
pub mod tokenizer;

#[cfg(feature = "tantivy")]
pub mod tantivy_reader;

#[cfg(feature = "tantivy")]
pub use tantivy_reader::TantivyIndexReader;

pub use memory::{InMemoryIndex, IndexedDocument};
pub use reader::{
    passes_filters, Candidate, DocId, DocumentMetadata, Explanation, FieldHits, FieldWeights,
    IndexError, IndexGeneration, SearchIndexReader, SearchOptions, TermExplanation,
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
