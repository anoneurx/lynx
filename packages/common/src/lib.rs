//! `lynx-common` — shared, dependency-light primitives.
//!
//! This crate holds primitives with no knowledge of LYNX business logic:
//! identifiers ([`ids`]), testable time access ([`time`]), the error taxonomy
//! ([`errors`]), hashing helpers ([`hash`]), URL normalisation ([`urlnorm`]),
//! Unicode-safe text handling ([`text`]), and byte budgets ([`bytesize`]).
//!
//! Invariants (see `packages/common/README.md`):
//! - no panics on `Result`-returning paths;
//! - all time access goes through a [`time::Clock`];
//! - `urlnorm` is deterministic and idempotent, property-tested.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

pub mod bytesize;
pub mod errors;
pub mod hash;
pub mod ids;
pub mod text;
pub mod time;
pub mod urlnorm;

pub use errors::{ErrorCode, LynxError};
