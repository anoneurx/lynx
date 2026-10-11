//! `lynx-indexer` — index generations.
//!
//! Builds a sealed Tantivy index generation from [`ParsedDocument`]s, with a manifest
//! recording shards and schema/ranking versions. Generations are sealed, then swapped
//! atomically.
//!
//! Stub established in implementation Step 1. The implementation lands in Step 4.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

/// The service name, used in logs and metrics.
pub const SERVICE_NAME: &str = "indexer";

#[cfg(test)]
mod tests {
    #[test]
    fn service_name_is_stable() {
        assert_eq!(super::SERVICE_NAME, "indexer");
    }
}
