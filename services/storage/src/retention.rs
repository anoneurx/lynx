//! Retention: the executable half of `retention_policy`.
//!
//! The schema (migration `0002_privacy_foundation.sql`) declares one row per
//! retention class, and its header comment states the rule: *"a table that cannot
//! name its class here does not get created."* Until now, that rule lived only in
//! review discipline; [`TABLES`] turns it into a test-enforced registry, and
//! [`sweep`] turns `retain_days` into actual deletes.
//!
//! Design decisions:
//!
//! * **Classes, not tables, carry the policy.** Operators tune `retention_policy`
//!   per class; the sweeper reads it at run time. A class that is missing from the
//!   policy table, or has `retain_days = NULL`, is **skipped**, never guessed —
//!   deleting by default is how data loss happens, and keeping forever by default
//!   is how the privacy promise is broken silently.
//! * **Governance tables are never swept.** `admin_audit_log` is append-only by
//!   trigger design; erasure requests follow
//!   [docs/privacy/legal.md](../../docs/privacy/legal.md) on the takedown path,
//!   not a bulk delete.
//! * **Idempotent and cheap by construction.** Every statement is a bounded
//!   `DELETE ... WHERE col < now() - interval`; running it twice in a row is a
//!   no-op the second time.

use std::time::Duration;

use sqlx::PgPool;

use crate::error::StorageError;

/// The retention classes declared in migration `0002_privacy_foundation.sql`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetentionClass {
    /// Life of the URL in the index; deleted by deindex/takedown, not by clock.
    CrawlPermanent,
    /// Operational crawl history — the diagnostic tail that goes stale.
    CrawlTemporary,
    /// Legal / anti-abuse records; never clock-swept.
    Governance,
    /// Metrics rollups.
    Aggregate,
    /// Contains no personal data by construction; clock sweep not needed.
    NoUserData,
}

impl RetentionClass {
    /// The `retention_policy.class` enum label this maps to in SQL.
    #[must_use]
    pub fn as_sql_enum(self) -> &'static str {
        match self {
            Self::CrawlPermanent => "crawl_permanent",
            Self::CrawlTemporary => "crawl_temporary",
            Self::Governance => "governance",
            Self::Aggregate => "aggregate",
            Self::NoUserData => "no_user_data",
        }
    }
}

/// One registered table and its retention fate.
///
/// `timed_column = Some(col)` means the sweeper may delete rows where `col` is
/// older than the class's `retain_days`. `None` means this table is never clock-
/// swept: it is permanent (deleted by domain action) or governance (untouchable).
#[derive(Debug, Clone, Copy)]
pub struct TableRetention {
    /// Table name as it appears in the migration SQL.
    pub table: &'static str,
    /// The retention class the table belongs to.
    pub class: RetentionClass,
    /// The timestamp/date column the sweeper may key on, when any.
    pub timed_column: Option<&'static str>,
    /// Why this fate is correct; the review checklist for every new table.
    pub note: &'static str,
}

// Every table created by `database/migrations/*.sql` MUST appear here exactly once
// (enforced by the `every_created_table_names_its_retention_fate` fence test in
// tests/retention.rs). A new table without a row in this registry — and without a
// matching `retention_policy` row in migration `0002`'s seed data — is a build
// failure. Retention is decided when the table is created, not after data exists.
pub const TABLES: &[TableRetention] = &[
    // ── migration meta ─────────────────────────────────────────────────────────
    TableRetention { table: "schema_migrations", class: RetentionClass::NoUserData, timed_column: None, note: "the migration ledger itself; must outlive the process" },
    TableRetention { table: "retention_policy", class: RetentionClass::NoUserData, timed_column: None, note: "the policy table drives sweeping; sweeping it would be circular" },
    // ── 0002 privacy foundation ────────────────────────────────────────────────
    TableRetention { table: "admin_audit_log", class: RetentionClass::Governance, timed_column: None, note: "append-only audit trail; never edited or deleted even by admin tooling" },
    // ── 0003 identities ────────────────────────────────────────────────────────
    TableRetention { table: "admin_user", class: RetentionClass::Governance, timed_column: None, note: "operator identities; only deprovisioning removes rows" },
    TableRetention { table: "admin_role", class: RetentionClass::Governance, timed_column: None, note: "role definitions" },
    TableRetention { table: "admin_user_role", class: RetentionClass::Governance, timed_column: None, note: "assignments follow the user / role lifecycle" },
    TableRetention { table: "api_key", class: RetentionClass::Governance, timed_column: None, note: "revocation is a domain action; the hash is never reversible" },
    TableRetention { table: "api_key_scope", class: RetentionClass::Governance, timed_column: None, note: "scopes follow the key lifecycle" },
    // ── 0004 crawl plane core ──────────────────────────────────────────────────
    TableRetention { table: "domain", class: RetentionClass::CrawlPermanent, timed_column: None, note: "life of the origin in the corpus" },
    TableRetention { table: "host", class: RetentionClass::CrawlPermanent, timed_column: None, note: "life of the origin in the corpus" },
    TableRetention { table: "url", class: RetentionClass::CrawlPermanent, timed_column: None, note: "life of the URL in the index; deindex/takedown deletes" },
    TableRetention { table: "crawl_frontier", class: RetentionClass::CrawlPermanent, timed_column: None, note: "the work queue: rows leave on successful claim, not on a clock" },
    TableRetention { table: "crawl_budget", class: RetentionClass::CrawlPermanent, timed_column: None, note: "per-host budgets are standing configuration" },
    // ── 0005 crawl plane ops ───────────────────────────────────────────────────
    TableRetention { table: "crawl_attempt", class: RetentionClass::CrawlTemporary, timed_column: Some("started_at"), note: "diagnostic crawl history; bound comes from retention_policy (dev seed: 90 days)" },
    TableRetention { table: "crawl_ban", class: RetentionClass::Governance, timed_column: None, note: "bans expire by their own `expires_at` in the enforcement path, not by sweep" },
    TableRetention { table: "crawl_stat_daily", class: RetentionClass::Aggregate, timed_column: Some("day"), note: "daily rollup: demanded by dashboards, worthless per-row after the retention window (dev seed: 13 months)" },
    // ── 0006 content ───────────────────────────────────────────────────────────
    TableRetention { table: "document", class: RetentionClass::CrawlPermanent, timed_column: None, note: "metadata for the indexed corpus" },
    TableRetention { table: "document_version", class: RetentionClass::CrawlPermanent, timed_column: None, note: "bounded per document; content lives in the index, not here" },
    TableRetention { table: "link", class: RetentionClass::CrawlPermanent, timed_column: None, note: "graph follows the URL lifecycle" },
    TableRetention { table: "robots_policy", class: RetentionClass::CrawlPermanent, timed_column: None, note: "raw robots.txt retained for compliance disputes" },
    // ── 0007 index bookkeeping ─────────────────────────────────────────────────
    TableRetention { table: "index_manifest", class: RetentionClass::NoUserData, timed_column: None, note: "generation bookkeeping; bounded by the keep-latest-N generation policy" },
    TableRetention { table: "index_shard", class: RetentionClass::NoUserData, timed_column: None, note: "follows the manifest lifecycle" },
    TableRetention { table: "index_segment", class: RetentionClass::NoUserData, timed_column: None, note: "follows the manifest lifecycle" },
    TableRetention { table: "dedup_cluster", class: RetentionClass::NoUserData, timed_column: None, note: "near-duplicate signatures; no user data" },
    // ── 0008 governance ────────────────────────────────────────────────────────
    TableRetention { table: "abuse_report", class: RetentionClass::Governance, timed_column: None, note: "the only personal data in Postgres (field-encrypted contact); erasure is actioned on request, never swept" },
    TableRetention { table: "takedown_request", class: RetentionClass::Governance, timed_column: None, note: "the durable record that content was removed — deleting the record would let a replay resurrect it" },
    // ── 0009 search quality ────────────────────────────────────────────────────
    TableRetention { table: "quality_evaluation_run", class: RetentionClass::NoUserData, timed_column: None, note: "run bookkeeping; kept for experiment comparison, no personal data" },
    TableRetention { table: "relevance_judgement", class: RetentionClass::NoUserData, timed_column: None, note: "the judged set is source-controlled corpus data" },
    // ── 0010 ranking config ────────────────────────────────────────────────────
    TableRetention { table: "ranking_config_version", class: RetentionClass::NoUserData, timed_column: None, note: "versioned ranking configuration; generated scores are reproducible from (doc, generation, config version)" },
];

/// The tables a timed sweep may act on: only the classes whose semantics are a
/// bounded operational history (diagnostics and rollups).
pub fn timed_tables() -> impl Iterator<Item = &'static TableRetention> {
    TABLES.iter().filter(|t| {
        t.timed_column.is_some()
            && matches!(
                t.class,
                RetentionClass::CrawlTemporary | RetentionClass::Aggregate
            )
    })
}

/// SQL for one timed delete. Rendered here (rather than inline) so a test can pin
/// the exact statement shape without a database.
///
/// `table`/`column` come from the const [`TABLES`] registry, never from input.
#[must_use]
pub fn render_sweep_statement(table: &str, column: &str) -> String {
    format!("DELETE FROM {table} WHERE {column} < now() - make_interval(days => $1)")
}

/// One per-table sweep outcome.
#[derive(Debug, Clone, Copy)]
pub struct SweepResult {
    /// The retention class label the table belongs to.
    pub class: &'static str,
    /// The table swept.
    pub table: &'static str,
    /// Rows deleted.
    pub deleted: u64,
}

/// Verify that every timed class has a deletable-by-clock policy.
///
/// Missing rows or `NULL` retain days are exactly the drift [`sweep`] skips by;
/// this function names them so a start-up check (or `lynx-db retention-check`)
/// can refuse to run a half-wired retention story.
///
/// # Errors
/// Returns [`StorageError`] on any SQL failure.
pub async fn verify_policy(pool: &PgPool) -> Result<Vec<String>, StorageError> {
    let mut violations = Vec::new();
    for retention in timed_tables() {
        if class_retain_days(pool, retention.class.as_sql_enum())
            .await?
            .is_none()
        {
            violations.push(format!(
                "retention_policy has no bound for class '{}' (tried table '{}'); \
                 add a row with retain_days before enabling the sweeper",
                retention.class.as_sql_enum(),
                retention.table
            ));
        }
    }
    Ok(violations)
}

/// Run one retention sweep pass over the operational schema.
///
/// Reads `retention_policy` live; a missing class or a `retain_days = NULL`
/// policy *skips* that class's tables (never guesses a fallback). Returns one
/// [`SweepResult`] per attempted table; skipped tables are simply absent from
/// the result set, and [`verify_policy`] is the tool that turns absence into a
/// loud, named finding.
///
/// # Errors
/// Returns [`StorageError`] on any SQL failure, which would in practice mean the
/// registry drifted away from the schema (unknown table/column).
pub async fn sweep(pool: &PgPool) -> Result<Vec<SweepResult>, StorageError> {
    let mut results = Vec::new();
    for retention in timed_tables() {
        let Some(days) = class_retain_days(pool, retention.class.as_sql_enum()).await? else {
            continue;
        };
        let Some(column) = retention.timed_column else {
            continue;
        };
        let sql = render_sweep_statement(retention.table, column);
        let deleted: u64 = sqlx::query(&sql)
            .bind(days)
            .execute(pool)
            .await?
            .rows_affected();
        results.push(SweepResult {
            class: retention.class.as_sql_enum(),
            table: retention.table,
            deleted,
        });
    }
    Ok(results)
}

/// How often the production sweeper runs, when [`spawn_daily_sweeper`] is used.
#[must_use]
pub fn sweep_interval() -> Duration {
    Duration::from_secs(24 * 60 * 60)
}

/// Spawn a background task that sweeps once per day (§Z Step 17, "retention jobs
/// live").
///
/// Errors are logged and never panic: a failed pass must not take the plane down,
/// and the next pass covers transient failures. The join handle is returned so a
/// supervisor can await shutdown in tests.
#[must_use]
pub fn spawn_daily_sweeper(pool: PgPool) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn(async move {
        let mut timer = tokio::time::interval(sweep_interval());
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            timer.tick().await;
            match sweep(&pool).await {
                Ok(results) => {
                    let deleted: u64 = results.iter().map(|r| r.deleted).sum();
                    tracing::info!(deleted, tables = results.len(), "retention sweep complete");
                }
                Err(err) => {
                    tracing::error!(%err, "retention sweep failed; next attempt in 24h");
                }
            }
        }
    })
}

/// Read `retain_days` for a class; `None` when the row is absent or `NULL`.
async fn class_retain_days(pool: &PgPool, class: &str) -> Result<Option<i32>, StorageError> {
    let row: Option<(Option<i32>,)> = match sqlx::query_as(
        "SELECT retain_days FROM retention_policy WHERE class = $1::retention_class",
    )
    .bind(class)
    .fetch_one(pool)
    .await
    {
        Ok(row) => Some(row),
        Err(sqlx::Error::RowNotFound) => None,
        Err(other) => return Err(other.into()),
    };
    Ok(row.and_then(|(days,)| days))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_statements_are_bounded_parameterised_deletes() {
        assert_eq!(
            render_sweep_statement("crawl_attempt", "started_at"),
            "DELETE FROM crawl_attempt WHERE started_at < now() - make_interval(days => $1)"
        );
    }

    #[test]
    fn timed_tables_are_only_the_diagnostic_and_rollup_ones() {
        let names: Vec<&str> = timed_tables().map(|t| t.table).collect();
        assert_eq!(names, vec!["crawl_attempt", "crawl_stat_daily"]);
    }

    #[test]
    fn every_timed_table_names_its_column_and_its_justification() {
        for t in timed_tables() {
            assert!(
                matches!(t.timed_column, Some("started_at") | Some("day")),
                "unplanned timed column on {}",
                t.table
            );
            assert!(
                t.note.len() > 10,
                "table {} must carry a written justification",
                t.table
            );
        }
    }

    #[test]
    fn classes_map_to_the_sql_enum_labels() {
        assert_eq!(
            RetentionClass::CrawlTemporary.as_sql_enum(),
            "crawl_temporary"
        );
        assert_eq!(RetentionClass::Governance.as_sql_enum(), "governance");
        assert_eq!(RetentionClass::Aggregate.as_sql_enum(), "aggregate");
    }

    #[test]
    fn the_interval_is_one_day() {
        assert_eq!(sweep_interval(), Duration::from_secs(86_400));
    }

    #[test]
    fn governance_tables_never_appear_in_the_timed_list() {
        for t in TABLES {
            if t.class == RetentionClass::Governance {
                assert!(
                    t.timed_column.is_none(),
                    "governance table {} must never be clock-swept",
                    t.table
                );
            }
        }
    }
}
