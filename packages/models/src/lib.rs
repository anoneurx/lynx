//! `lynx-models` — domain types and the public API contract.
//!
//! Two layers, deliberately separate:
//!
//! - [`domain`] — internal types shared by the query and crawl planes.
//! - [`api`] — the public, reviewed narrowing of those types. An internal refactor
//!   must not be able to leak a new field onto the internet.
//!
//! Rules (see `packages/models/README.md`):
//! 1. No user-data types exist here. There is no `User`, `Session`, or `SearchHistory`.
//! 2. Types crossing a boundary are version-tagged so rolling upgrades are safe.
//! 3. Public types serialise an explicit allow-list of fields.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

pub mod api;
pub mod domain;

pub use domain::{Document, DocumentVersion, Host, Url, UrlId};
