//! Pooled connection handling.

use std::fmt;
use std::path::Path;

use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::error::StorageError;
use crate::migrations::{self, Migration};

/// A handle to the operational Postgres database.
///
/// One pool per process. The database role behind `url` determines what the process
/// may do: the API connects with a read-only role, the crawler with a narrower
/// write role, and migrations with the schema owner. Least privilege is enforced by
/// grants, not convention (`docs/database/README.md`).
#[derive(Clone)]
pub struct Storage {
    pool: PgPool,
}

impl Storage {
    /// Connect with an explicit connection ceiling.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Database`] if the pool cannot connect.
    pub async fn connect(url: &str, max_connections: u32) -> Result<Self, StorageError> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .connect(url)
            .await?;
        Ok(Self { pool })
    }

    /// The underlying pool, for callers that need a raw connection.
    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Apply every pending migration in `dir`, returning the versions applied.
    ///
    /// # Errors
    ///
    /// Propagates [`StorageError`] from migration discovery, planning, and execution.
    pub async fn migrate(&self, dir: &Path) -> Result<Vec<u32>, StorageError> {
        migrations::run(&self.pool, dir).await
    }

    /// The migrations on disk that have not been applied, without applying them.
    ///
    /// # Errors
    ///
    /// Propagates [`StorageError`] from discovery and from reading the ledger.
    pub async fn pending_migrations(&self, dir: &Path) -> Result<Vec<Migration>, StorageError> {
        let discovered = migrations::discover(dir)?;
        let applied = migrations::load_applied(&self.pool).await?;
        migrations::plan(&applied, &discovered)
    }

    /// Close the pool, waiting for in-flight queries to finish.
    pub async fn close(self) {
        self.pool.close().await;
    }
}

impl fmt::Debug for Storage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Storage")
            .field("pool", &self.pool)
            .finish_non_exhaustive()
    }
}
