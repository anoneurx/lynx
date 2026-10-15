//! Migration discovery, drift detection, and application.
//!
//! Migrations are forward-only and append-only (`database/migrations/README.md`).
//! The runner reads `NNNN_name.sql` files in ascending version order, records an
//! applied checksum in `schema_migrations`, and refuses to run if an already-applied
//! file has changed. The discovery and planning steps are pure and are unit-tested
//! without a database.

use std::collections::BTreeMap;
use std::path::Path;

use sqlx::PgPool;

use crate::error::StorageError;

/// One migration file on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migration {
    /// Numeric version parsed from the file name.
    pub version: u32,
    /// Descriptive name parsed from the file name.
    pub name: String,
    /// The SQL body.
    pub sql: String,
    /// Lower-case hex BLAKE3 digest of `sql`.
    pub checksum: String,
}

/// A migration already recorded in `schema_migrations`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedMigration {
    /// Numeric version.
    pub version: u32,
    /// Recorded name.
    pub name: String,
    /// Recorded checksum.
    pub checksum: String,
}

/// The ledger table, created idempotently before the ledger is read.
const LEDGER_DDL: &str = "
CREATE TABLE IF NOT EXISTS schema_migrations (
  version    integer     PRIMARY KEY,
  name       text        NOT NULL,
  checksum   text        NOT NULL,
  applied_at timestamptz NOT NULL DEFAULT now()
)";

/// The lower-case hex BLAKE3 checksum of a migration body.
#[must_use]
pub fn checksum(sql: &str) -> String {
    blake3::hash(sql.as_bytes()).to_hex().to_string()
}

/// Parse `NNNN_name.sql` into `(version, name)`, or `None` if the shape is wrong.
///
/// The version is exactly four ASCII digits; the name is everything after the first
/// underscore, with the `.sql` suffix removed.
#[must_use]
pub fn parse_file_name(file_name: &str) -> Option<(u32, String)> {
    let stem = file_name.strip_suffix(".sql")?;
    let (prefix, name) = stem.split_once('_')?;
    if prefix.len() != 4 || !prefix.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let version = prefix.parse::<u32>().ok()?;
    Some((version, name.to_string()))
}

/// Read every `.sql` migration in `dir`, sorted ascending by version.
///
/// # Errors
///
/// Returns [`StorageError::Io`] if the directory or a file cannot be read,
/// [`StorageError::BadMigrationName`] for a misnamed file, and
/// [`StorageError::DuplicateVersion`] if two files share a version.
pub fn discover(dir: &Path) -> Result<Vec<Migration>, StorageError> {
    let entries = std::fs::read_dir(dir).map_err(|source| StorageError::Io {
        path: dir.to_path_buf(),
        source,
    })?;

    let mut migrations = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| StorageError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(std::ffi::OsStr::to_str) != Some("sql") {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default()
            .to_string();
        let Some((version, name)) = parse_file_name(&file_name) else {
            return Err(StorageError::BadMigrationName(file_name));
        };
        let sql = std::fs::read_to_string(&path).map_err(|source| StorageError::Io {
            path: path.clone(),
            source,
        })?;
        let checksum = checksum(&sql);
        migrations.push(Migration {
            version,
            name,
            sql,
            checksum,
        });
    }

    migrations.sort_by_key(|m| m.version);
    for pair in migrations.windows(2) {
        if pair[0].version == pair[1].version {
            return Err(StorageError::DuplicateVersion(pair[0].version));
        }
    }
    Ok(migrations)
}

/// Given what is applied and what is on disk, return the pending migrations.
///
/// # Errors
///
/// Returns [`StorageError::ChecksumMismatch`] if an applied migration's file has
/// changed since it was applied.
pub fn plan(
    applied: &[AppliedMigration],
    discovered: &[Migration],
) -> Result<Vec<Migration>, StorageError> {
    let index: BTreeMap<u32, &AppliedMigration> = applied.iter().map(|a| (a.version, a)).collect();

    let mut pending = Vec::new();
    for migration in discovered {
        match index.get(&migration.version) {
            Some(previous) if previous.checksum != migration.checksum => {
                return Err(StorageError::ChecksumMismatch {
                    version: migration.version,
                    applied: previous.checksum.clone(),
                    found: migration.checksum.clone(),
                });
            }
            Some(_) => {}
            None => pending.push(migration.clone()),
        }
    }
    Ok(pending)
}

/// Load the applied ledger, oldest first.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the query fails.
pub async fn load_applied(pool: &PgPool) -> Result<Vec<AppliedMigration>, StorageError> {
    let rows = sqlx::query_as::<_, LedgerRow>(
        "SELECT version, name, checksum FROM schema_migrations ORDER BY version",
    )
    .fetch_all(pool)
    .await?;

    let mut applied = Vec::with_capacity(rows.len());
    for row in rows {
        applied.push(AppliedMigration {
            version: u32::try_from(row.version).map_err(|_| StorageError::Represent {
                column: "schema_migrations.version",
                value: row.version.to_string(),
            })?,
            name: row.name,
            checksum: row.checksum,
        });
    }
    Ok(applied)
}

#[derive(sqlx::FromRow)]
struct LedgerRow {
    version: i32,
    name: String,
    checksum: String,
}

/// Apply all pending migrations in `dir` and return the versions applied.
///
/// Each migration runs in its own transaction and records its checksum on success,
/// so a failure halfway leaves the ledger consistent and the run resumable.
///
/// # Errors
///
/// Propagates discovery errors ([`discover`], [`plan`]) and
/// [`StorageError::Database`] from the DDL, the migration bodies, or the ledger write.
pub async fn run(pool: &PgPool, dir: &Path) -> Result<Vec<u32>, StorageError> {
    sqlx::raw_sql(LEDGER_DDL).execute(pool).await?;

    let discovered = discover(dir)?;
    let applied = load_applied(pool).await?;
    let pending = plan(&applied, &discovered)?;

    let mut done = Vec::with_capacity(pending.len());
    for migration in pending {
        let version = i32::try_from(migration.version).map_err(|_| StorageError::Represent {
            column: "schema_migrations.version",
            value: migration.version.to_string(),
        })?;

        let mut tx = pool.begin().await?;
        sqlx::raw_sql(&migration.sql).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO schema_migrations (version, name, checksum) VALUES ($1, $2, $3)")
            .bind(version)
            .bind(&migration.name)
            .bind(&migration.checksum)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;

        done.push(migration.version);
    }
    Ok(done)
}
