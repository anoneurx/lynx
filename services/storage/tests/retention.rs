//! The retention fence: every table the migrations create must name its fate.
//!
//! Migration `0002_privacy_foundation.sql` states the rule in prose: *"a table
//! that cannot name its class here does not get created."* These tests make that
//! prose executable: `lynx_storage::retention::TABLES` must list every
//! `CREATE TABLE` in `database/migrations/` exactly once, with a class and — for
//! clock-swept tables — a column that actually exists in that table's DDL.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use lynx_storage::retention::{
    render_sweep_statement, timed_tables, RetentionClass, TableRetention, TABLES,
};

fn repo_migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../database/migrations")
}

/// Extract every base-table name from the migration SQL, excluding partition
/// defaults (`X_default PARTITION OF X`) and index/truncate/enum statements.
fn created_tables_from_migrations() -> Vec<(u32, String)> {
    let dir = repo_migrations_dir();
    let mut found = Vec::new();
    for file in dir.read_dir().unwrap() {
        let path = file.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let Some((version, _)) = parse_version(&name) else {
            continue;
        };
        let sql = std::fs::read_to_string(&path).unwrap();
        let normalized = sql.replace("CREATE TABLE IF NOT EXISTS ", "CREATE TABLE ");
        for line in normalized.lines() {
            let rest = line.trim_start();
            let Some(table_part) = rest.strip_prefix("CREATE TABLE ") else {
                continue;
            };
            let table = table_part
                .split(|c: char| c.is_whitespace() || c == '(')
                .next()
                .unwrap();
            if table_part.contains("PARTITION OF") {
                continue; // a partition of an already-registered parent
            }
            found.push((version, table.to_string()));
        }
    }
    found.sort();
    found.dedup();
    found
}

fn parse_version(name: &str) -> Option<(u32, String)> {
    let (num, rest) = name.split_once('_')?;
    num.parse::<u32>().ok().map(|v| (v, rest.to_string()))
}

#[test]
fn every_created_table_names_its_retention_fate() {
    let created = created_tables_from_migrations();
    assert!(
        created.len() > 20,
        "fence sanity: the extraction should find real tables, got {}",
        created.len()
    );
    let registered: BTreeSet<&str> = TABLES.iter().map(|t| t.table).collect();
    let mut problems = Vec::new();
    for (version, table) in &created {
        if !registered.contains(table.as_str()) {
            problems.push(format!(
                "migration {version} creates table '{table}' but retention::TABLES has \
                 no row for it: name its class when you create it, not later"
            ));
        }
    }
    // And the reverse: a row for a table the schema never created is dead weight
    // (or a typo) that would drift the registry from reality.
    let created_names: BTreeSet<&str> = created.iter().map(|(_, t)| t.as_str()).collect();
    for t in TABLES {
        if !created_names.contains(t.table) {
            problems.push(format!(
                "retention::TABLES lists '{}' but no migration creates that table",
                t.table
            ));
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

#[test]
fn registered_tables_are_unique() {
    let mut seen = Vec::new();
    for t in TABLES {
        assert!(
            !seen.contains(&t.table),
            "'{}' appears twice in retention::TABLES",
            t.table
        );
        seen.push(t.table);
    }
}

#[test]
fn every_table_carries_a_written_justification() {
    for t in TABLES {
        assert!(
            t.note.len() >= 15,
            "table {} needs a real 'why', not a stub ({:?})",
            t.table,
            t.note
        );
    }
}

#[test]
fn timed_columns_exist_in_their_migrations_ddl() {
    // For each timed table, find its migration's DDL and require the named
    // column to appear inside the CREATE TABLE block.
    for t in timed_tables() {
        let col = t.timed_column.expect("timed by the filter");
        let block = create_table_block(t.table)
            .unwrap_or_else(|| panic!("no CREATE TABLE block found for timed table {}", t.table));
        assert!(
            block.contains(&format!("{col}             "))
                || block.contains(&format!("{col}   "))
                || block.lines().any(|l| l.trim_start().starts_with(col)),
            "timed table {} names column '{col}' but its DDL has no such column",
            t.table
        );
    }
}

fn create_table_block(table: &str) -> Option<String> {
    let dir = repo_migrations_dir();
    for file in dir.read_dir().unwrap() {
        let path = file.unwrap().path();
        let sql = std::fs::read_to_string(&path).unwrap();
        if let Some(start) = sql.find(&format!("CREATE TABLE IF NOT EXISTS {table} (")) {
            let end = sql[start..].find(");").map_or(sql.len(), |o| start + o);
            return Some(sql[start..end].to_string());
        }
    }
    None
}

#[test]
fn governance_tables_are_never_clock_swept() {
    for t in TABLES {
        if t.class == RetentionClass::Governance {
            assert!(
                t.timed_column.is_none(),
                "governance table {} must not carry a timed column",
                t.table
            );
        }
        if t.class == RetentionClass::CrawlPermanent {
            assert!(
                t.timed_column.is_none(),
                "crawl_permanent table {} deletion is domain-driven, not clock-driven",
                t.table
            );
        }
    }
}

#[test]
fn the_registry_is_the_only_source_for_the_sweep() {
    // Sweep statements must be derivable exactly from the registry; nothing else
    // is allowed to name tables for deletion.
    let expected: Vec<String> = timed_tables()
        .map(|t: &TableRetention| render_sweep_statement(t.table, t.timed_column.unwrap()))
        .collect();
    assert_eq!(
        expected,
        vec![
            "DELETE FROM crawl_attempt WHERE started_at < now() - make_interval(days => $1)"
                .to_string(),
            "DELETE FROM crawl_stat_daily WHERE day < now() - make_interval(days => $1)"
                .to_string(),
        ]
    );
}

#[test]
fn the_seed_policies_cover_every_class_the_sweeper_might_read() {
    // The sweeper reads retention_policy at run time. Every class label it can
    // name must therefore exist in the shipped seed, byte-exact, so a masked
    // typo in the enum label cannot silently skip a sweep.
    let seed = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../database/seeds/seed.sql"),
    )
    .expect("seed.sql is a repo file");
    for class in [
        RetentionClass::CrawlPermanent,
        RetentionClass::CrawlTemporary,
        RetentionClass::Governance,
        RetentionClass::Aggregate,
        RetentionClass::NoUserData,
    ] {
        let label = class.as_sql_enum();
        assert!(
            seed.contains(&format!("('{label}'")),
            "seed.sql has no retention_policy row for class '{label}'"
        );
    }
}
