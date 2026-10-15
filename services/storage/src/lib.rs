//! `lynx-storage` — the Postgres operational store.
//!
//! Postgres holds what Tantivy cannot: mutable state, relationships, and queues. It
//! holds no search index and no user data, and keeping that boundary sharp is what
//! makes the privacy architecture auditable (`docs/database/README.md`).
//!
//! This crate has three responsibilities:
//!
//! - [`migrations`] — discover `NNNN_name.sql` files, detect drift against the
//!   `schema_migrations` ledger, and apply pending migrations transactionally.
//! - [`pool`] — a [`Storage`](pool::Storage) handle wrapping a bounded `PgPool`.
//! - [`repos`] — explicit, reviewable queries returning [`rows`] types.
//!
//! The physical schema is defined by the SQL in `database/migrations`, and the
//! human-readable contract is in `database/schemas`. Neither the SQL nor the schema
//! docs are generated from this crate; this crate reads the schema, it does not own
//! it.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

pub mod error;
pub mod migrations;
pub mod pool;
pub mod repos;
pub mod rows;

pub use error::StorageError;
pub use migrations::{AppliedMigration, Migration};
pub use pool::Storage;
