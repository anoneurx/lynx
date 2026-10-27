//! `lynx-security` — reusable, auditable security controls.
//!
//! Every module here is security-critical and carries two-reviewer requirements on
//! change (see `packages/security/README.md`). Modules:
//!
//! - [`ratelimit`] — rotating salted prefix buckets.
//! - [`apikey`] — API key generation, storage, and constant-time verification.
//! - [`headers`] — the canonical security header set.
//! - [`blocklist`] — host/domain matching for bans and policy terms.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

pub mod apikey;
pub mod blocklist;
pub mod headers;
pub mod privacy;
pub mod ratelimit;
