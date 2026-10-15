//! Migration discovery, naming, checksum, and drift tests.
//!
//! These exercise the pure planning logic against the real `database/migrations`
//! directory and against synthetic files, without a database.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use lynx_storage::migrations::{
    checksum, discover, parse_file_name, plan, AppliedMigration, Migration,
};
use lynx_storage::StorageError;

fn repo_migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../database/migrations")
}

#[test]
fn parses_a_well_formed_name() {
    assert_eq!(
        parse_file_name("0004_crawl_plane_core.sql"),
        Some((4, "crawl_plane_core".to_string()))
    );
}

#[test]
fn rejects_malformed_names() {
    assert_eq!(parse_file_name("crawl_plane_core.sql"), None);
    assert_eq!(parse_file_name("4_crawl.sql"), None);
    assert_eq!(parse_file_name("abcd_thing.sql"), None);
    assert_eq!(parse_file_name("0004_crawl_plane_core"), None);
}

#[test]
fn checksum_is_deterministic_and_content_sensitive() {
    assert_eq!(checksum("SELECT 1;"), checksum("SELECT 1;"));
    assert_ne!(checksum("SELECT 1;"), checksum("SELECT 2;"));
}

#[test]
fn discovers_the_repository_migrations_in_order() {
    let migrations = discover(&repo_migrations_dir()).expect("discover real migrations");
    let versions: Vec<u32> = migrations.iter().map(|m| m.version).collect();
    assert_eq!(versions, (1..=10).collect::<Vec<u32>>());
    assert!(migrations.iter().all(|m| !m.sql.is_empty()));
    // Every migration declares its expand/contract tag.
    assert!(migrations.iter().all(|m| m.sql.contains("+migrate:")));
}

#[test]
fn rejects_duplicate_versions() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("0001_a.sql"), "SELECT 1;").unwrap();
    std::fs::write(dir.path().join("0001_b.sql"), "SELECT 2;").unwrap();
    match discover(dir.path()) {
        Err(StorageError::DuplicateVersion(1)) => {}
        other => panic!("expected DuplicateVersion(1), got {other:?}"),
    }
}

#[test]
fn rejects_a_misnamed_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("oops.sql"), "SELECT 1;").unwrap();
    assert!(matches!(
        discover(dir.path()),
        Err(StorageError::BadMigrationName(_))
    ));
}

#[test]
fn ignores_non_sql_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("README.md"), "notes").unwrap();
    std::fs::write(dir.path().join("0001_a.sql"), "SELECT 1;").unwrap();
    let migrations = discover(dir.path()).unwrap();
    assert_eq!(migrations.len(), 1);
    assert_eq!(migrations[0].version, 1);
}

#[test]
fn plan_returns_unapplied_migrations() {
    let discovered = vec![
        Migration {
            version: 1,
            name: "a".into(),
            sql: "SELECT 1;".into(),
            checksum: checksum("SELECT 1;"),
        },
        Migration {
            version: 2,
            name: "b".into(),
            sql: "SELECT 2;".into(),
            checksum: checksum("SELECT 2;"),
        },
    ];
    let applied = vec![AppliedMigration {
        version: 1,
        name: "a".into(),
        checksum: checksum("SELECT 1;"),
    }];

    let pending = plan(&applied, &discovered).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].version, 2);
}

#[test]
fn plan_detects_drift_on_an_applied_migration() {
    let discovered = vec![Migration {
        version: 1,
        name: "a".into(),
        sql: "SELECT 1;".into(),
        checksum: checksum("SELECT 1;"),
    }];
    let applied = vec![AppliedMigration {
        version: 1,
        name: "a".into(),
        checksum: checksum("SELECT 999;"), // different content than the file
    }];

    match plan(&applied, &discovered) {
        Err(StorageError::ChecksumMismatch { version, .. }) => assert_eq!(version, 1),
        other => panic!("expected ChecksumMismatch, got {other:?}"),
    }
}

#[test]
fn plan_is_empty_when_everything_is_applied() {
    let discovered = vec![Migration {
        version: 1,
        name: "a".into(),
        sql: "SELECT 1;".into(),
        checksum: checksum("SELECT 1;"),
    }];
    let applied = vec![AppliedMigration {
        version: 1,
        name: "a".into(),
        checksum: checksum("SELECT 1;"),
    }];
    assert!(plan(&applied, &discovered).unwrap().is_empty());
}
