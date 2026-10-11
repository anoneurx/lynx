//! `lynx-suggestions` — query completions.
//!
//! Completions are derived from index terms and document titles in the corpus. There
//! is no query log to mine, by design.
//!
//! Stub established in implementation Step 1.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

/// The service name, used in logs and metrics.
pub const SERVICE_NAME: &str = "suggestions";

#[cfg(test)]
mod tests {
    #[test]
    fn service_name_is_stable() {
        assert_eq!(super::SERVICE_NAME, "suggestions");
    }
}
