//! `lynx-parser` — HTML extraction and sanitisation.
//!
//! Produces a bounded [`ParsedDocument`](lynx_models::domain::ParsedDocument): title,
//! headings, cleaned body, anchors, and outbound links. No script, no markup, no
//! unbounded text reaches the index.
//!
//! Stub established in implementation Step 1.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

/// The service name, used in logs and metrics.
pub const SERVICE_NAME: &str = "parser";

#[cfg(test)]
mod tests {
    #[test]
    fn service_name_is_stable() {
        assert_eq!(super::SERVICE_NAME, "parser");
    }
}
