//! Index bootstrap: what the serving process actually reads from.
//!
//! One sealed generation on disk (ADR-0004) is the real state; the in-memory
//! fixture exists so `cargo run` in development works with no crawl at all. The
//! selection is code, not env-var magic: a production boot with no index
//! manifest is a boot failure with an operator-readable reason, never a silent
//! empty search surface.

use std::path::Path;
use std::sync::Arc;

use lynx_indexer::{InMemoryIndex, IndexedDocument, SearchIndexReader, TantivyIndexReader};
use time::macros::datetime;
use tracing::info;

/// Which reader the process serves from; carries its provenance for the boot log.
pub enum ServingIndex {
    /// A sealed Tantivy generation from `index.dir`.
    Tantivy(Arc<dyn SearchIndexReader>, u64),
    /// The development-only seeded fixture.
    Fixture(Arc<dyn SearchIndexReader>),
}

impl std::fmt::Debug for ServingIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tantivy(_, doc_count) => f
                .debug_tuple("ServingIndex::Tantivy")
                .field(doc_count)
                .finish(),
            Self::Fixture(_) => f.write_str("ServingIndex::Fixture"),
        }
    }
}

impl ServingIndex {
    /// The reader to serve.
    #[must_use]
    pub fn reader(&self) -> Arc<dyn SearchIndexReader> {
        match self {
            Self::Tantivy(reader, _) | Self::Fixture(reader) => reader.clone(),
        }
    }

    /// Human line for the boot log; names the source.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Tantivy(_, doc_count) => {
                format!("sealed tantivy generation: {doc_count} documents")
            }
            Self::Fixture(_) => "development fixture (seeded in-memory index)".to_string(),
        }
    }
}

/// Choose the serving index for this boot.
///
/// - `index.dir` holding a generation manifest (`generation.json`) wins: the
///   process serves what the crawler built, whatever the environment.
/// - Otherwise, development and staging boot the seeded fixture so the API is
///   usable with zero infrastructure.
/// - Otherwise — a production boot with no index — this is a hard error. Serving
///   an empty search to users *is* the outage by another name.
///
/// # Errors
/// [`IndexError`]-backed formatted string when the manifest exists but the index
/// cannot open, or when production has no generation to serve.
pub fn open(config: &lynx_config::Config) -> Result<ServingIndex, String> {
    let dir = Path::new(&config.index.dir);
    let manifest = dir.join("generation.json");
    if manifest.exists() {
        let reader = TantivyIndexReader::open(dir)
            .map_err(|e| format!("cannot open the index generation in {}: {e}", dir.display()))?;
        let doc_count = reader.generation().doc_count;
        info!(
            service = "lynx-api",
            dir = %dir.display(),
            doc_count,
            "opened index generation"
        );
        return Ok(ServingIndex::Tantivy(Arc::new(reader), doc_count));
    }

    if config.core.environment.is_production() {
        return Err(format!(
            "production requires a sealed index generation in index.dir ({}): found none. \
             Run the crawler to produce one before starting the public API; \
             serving an empty index to users would be the outage by another name",
            dir.display()
        ));
    }

    info!(
        service = "lynx-api",
        dir = %dir.display(),
        "no index generation present; serving the development fixture"
    );
    Ok(ServingIndex::Fixture(Arc::new(development_fixture())))
}

/// The seeded development fixture: small, hand-written, representative — the same
/// corpus `apps/api/tests` films for quality behaviours. Temporary by design: it
/// disappears when a crawl exists.
#[must_use]
pub fn development_fixture() -> InMemoryIndex {
    let mut index = InMemoryIndex::new();
    let mut rust_doc = IndexedDocument::new(
        lynx_common::ids::DocumentId::from_canonical_url("https://tokio.rs/"),
        "https://tokio.rs/",
        "Tokio - An asynchronous runtime for Rust",
        "A multi-threaded asynchronous runtime for Rust, with a scheduler, timers, and an I/O driver.",
    );
    rust_doc.lang = Some("en".to_string());
    rust_doc.published_at = Some(datetime!(2025-11-04 00:00 UTC));
    rust_doc.quality_score = 0.9;
    index.add(rust_doc);

    let mut ownership = IndexedDocument::new(
        lynx_common::ids::DocumentId::from_canonical_url(
            "https://doc.rust-lang.org/book/ch04-00-understanding-ownership/",
        ),
        "https://doc.rust-lang.org/book/ch04-00-understanding-ownership/",
        "Understanding Ownership - The Rust Programming Language",
        "Ownership is a set of rules that govern how a Rust program manages memory. \
         When the owner goes out of scope, the value is dropped.",
    );
    ownership.lang = Some("en".to_string());
    ownership.published_at = Some(datetime!(2024-06-01 00:00 UTC));
    ownership.quality_score = 0.85;
    index.add(ownership);

    index
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use tempfile::tempdir;

    fn config_with(dir: &str, production: bool) -> lynx_config::Config {
        let mut config =
            lynx_config::Config::from_toml_str(include_str!("../../../config/example.toml"))
                .unwrap();
        config.index.dir = dir.to_string();
        if production {
            config.core.environment = lynx_config::Environment::Production;
            config.core.instance_id = "test-1".into();
            config.admin.enable_basic_auth = false;
            config.ai.enabled = false;
        }
        config
    }

    fn generation() -> lynx_indexer::IndexGeneration {
        lynx_indexer::IndexGeneration {
            id: 1,
            tokenizer_version: "lynx-test".to_string(),
            ranking_config_version: "v0.1.0".to_string(),
            created_at: datetime!(2026-03-01 00:00 UTC),
            doc_count: 0,
            size_bytes: 0,
            checksum: String::new(),
            shards: 1,
        }
    }

    #[test]
    fn a_sealed_generation_is_served_in_any_environment() {
        let dir = tempdir().unwrap();
        let doc = IndexedDocument::new(
            lynx_common::ids::DocumentId::from_canonical_url("https://tokio.rs/"),
            "https://tokio.rs/",
            "Tokio",
            "A multi-threaded asynchronous runtime for Rust.",
        );
        TantivyIndexReader::write_to_dir(dir.path(), [doc], &generation()).unwrap();

        let serving = open(&config_with(&dir.path().display().to_string(), false)).unwrap();
        let ServingIndex::Tantivy(_, doc_count) = &serving else {
            panic!("a sealed generation must win over the fixture")
        };
        assert_eq!(*doc_count, 1);

        // And production is served the same way.
        let serving = open(&config_with(&dir.path().display().to_string(), true)).unwrap();
        assert!(matches!(serving, ServingIndex::Tantivy(_, _)));
    }

    #[test]
    fn a_missing_generation_falls_back_in_development_only() {
        let dir = tempdir().unwrap();
        let serving = open(&config_with(&dir.path().display().to_string(), false)).unwrap();
        let ServingIndex::Fixture(reader) = &serving else {
            panic!("development falls back to the fixture")
        };
        assert!(
            reader
                .count(&lynx_query::parse("rust"), datetime!(2026-03-01 00:00 UTC))
                .unwrap()
                >= 1
        );
    }

    #[test]
    fn production_refuses_to_boot_without_an_index() {
        let dir = tempdir().unwrap();
        let err = open(&config_with(&dir.path().display().to_string(), true))
            .expect_err("empty production must not boot");
        assert!(err.contains("sealed index generation"), "{err}");
    }
}
