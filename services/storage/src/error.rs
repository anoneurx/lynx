//! Errors for the storage crate.

use std::path::PathBuf;

/// Anything that can go wrong while talking to Postgres or applying migrations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StorageError {
    /// A filesystem error while reading a migration directory or file.
    #[error("i/o error at {path}: {source}")]
    Io {
        /// The path being read.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },

    /// A migration file was not named `NNNN_name.sql`.
    #[error("migration file name is not NNNN_name.sql: {0}")]
    BadMigrationName(String),

    /// Two migration files declared the same version.
    #[error("duplicate migration version {0}")]
    DuplicateVersion(u32),

    /// An applied migration's content no longer matches the file on disk (drift).
    ///
    /// Editing a shipped migration makes environments diverge permanently, so this
    /// is a hard failure rather than a warning.
    #[error("migration {version} checksum mismatch: applied {applied}, found {found}")]
    ChecksumMismatch {
        /// The version that diverged.
        version: u32,
        /// The checksum recorded when it was applied.
        applied: String,
        /// The checksum of the file on disk now.
        found: String,
    },

    /// A value could not be represented in the expected column shape.
    #[error("could not represent {column}: {value}")]
    Represent {
        /// The column name.
        column: &'static str,
        /// The offending value, rendered.
        value: String,
    },

    /// The database rejected a statement or connection.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
