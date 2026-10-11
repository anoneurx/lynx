//! `lynx-ai` — the optional answer layer.
//!
//! Off by default. When enabled, it may summarise retrieved sources but must cite them,
//! and fails closed if citations cannot be verified. It never sees a query it cannot
//! ground in retrieved, citable documents.
//!
//! Stub established in implementation Step 1.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

/// The service name, used in logs and metrics.
pub const SERVICE_NAME: &str = "ai";

#[cfg(test)]
mod tests {
    #[test]
    fn service_name_is_stable() {
        assert_eq!(super::SERVICE_NAME, "ai");
    }
}
