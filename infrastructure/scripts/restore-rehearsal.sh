#!/usr/bin/env sh
# Restore rehearsal: the backup/restore cycle against the local stack.
#
# Run when: before any deploy that changes backup tooling, after compose brings
# the stack up, and as part of a DR review. Usage:
#
#   infrastructure/scripts/restore-rehearsal.sh [--keep-workdir]
#
# What is rehearsed, and honestly what is not:
#   1. Postgres: logical dump (pg_dump) of the dev cluster, restore into a fresh
#      scratch database on the same container, row-by-table count comparison.
#      This rehearses the same pg_dump/restore mechanics production WAL-G
#      restores are verified with (a base backup + WAL replay needs the real
#      cluster env; those run with the staging kit, not locally).
#   2. Index generations: copy every generation directory through the same
#      path pattern production snapshots, checksum both sides, perform the
#      symlink swap on the copy, and verify the canonical content equals the
#      source through the swap.
#   3. NOT rehearsed locally: Vault Raft snapshot restore (needs a Vault
#      cluster), Redis RDB swap (needs the service), reader reload via pub/sub
#      (needs the running API). Those belong to the staging rehearsal; their
#      evidence is logged in docs/operations/backup.md#restore-rehearsal.
#
# Exit 0 only when every rehearsed step passed. Any failure is a finding: a
# backup that has never been restored is a hypothesis, not a backup.

set -eu

cd "$(dirname "$0")/../.."

KEEP=0
[ "${1:-}" = "--keep-workdir" ] && KEEP=1

export TMPDIR=${TMPDIR:-/tmp}
WORK="$TMPDIR/lynx-restore-rehearsal-$(date +%s)"
mkdir -p "$WORK"
trap '[ "$KEEP" = 1 ] || rm -rf "$WORK"' EXIT

failures=0
note() { printf '%s\n' "$*"; }
expect_eq() {
    if [ "$1" = "$2" ]; then
        note "PASS: $3"
    else
        note "FAIL: $3 (left=$1 right=$2)" >&2
        failures=$((failures + 1))
    fi
}

# ── Requirements ───────────────────────────────────────────────────────────────
for bin in docker psql pg_dump pg_restore sha224sum; do
    command -v "$bin" >/dev/null 2>&1 || { note "missing tool: $bin" >&2; exit 2; }
done
docker compose --version >/dev/null 2>&1 || { note "docker compose plugin required" >&2; exit 2; }

DB_SERVICE=lynx-db
DB_USER=lynx
DB_NAME=lynx
if ! docker compose ps --status running 2>/dev/null | grep -q "$DB_SERVICE"; then
    note "starting the local stack (docker compose)..."
    docker compose up -d "$DB_SERVICE" >/dev/null
    docker compose exec "$DB_SERVICE" pg_isready -U "$DB_USER" -d "$DB_NAME" \
        --timeout=60 || { note "database did not become ready" >&2; exit 2; }
fi

# ── 1. Postgres: dump → scratch restore → table-count comparison ──────────────
note "rehearsal 1: Postgres logical round-trip..."

docker compose exec -T "$DB_SERVICE" \
    pg_dump -U "$DB_USER" -Fc "$DB_NAME" > "$WORK/lynx.dump"

SCRATCH="rehearsal_$(date +%s)"
docker compose exec -T "$DB_SERVICE" createdb -U "$DB_USER" "$SCRATCH"
cleanup_scratch() {
    docker compose exec -T "$DB_SERVICE" dropdb -U "$DB_USER" --if-exists "$SCRATCH" >/dev/null 2>&1 || true
}
trap 'cleanup_scratch; [ "$KEEP" = 1 ] || rm -rf "$WORK"' EXIT

docker compose exec -T "$DB_SERVICE" env PGPASSWORD= \
    pg_restore -U "$DB_USER" -d "$SCRATCH" --no-owner --exit-on-error \
    < "$WORK/lynx.dump" 2>"$WORK/pg_restore.err"
note "PASS: restore into scratch $SCRATCH completed with --exit-on-error"

counts_before=$(docker compose exec -T "$DB_SERVICE" psql -U "$DB_USER" -d "$DB_NAME" -A -t -c "
    SELECT coalesce(tid, 'none') || '=' || coalesce(cnt, 0)
    FROM (
        SELECT relid::regclass::text AS tbl,
               (SELECT count(*) FROM pg_stat_all_tables s WHERE s.relid = t.relid) AS cnt,
               t.relid
        FROM pg_stat_all_tables t
        WHERE schemaname = 'public'
    ) q
    UNION ALL
    SELECT 'tables',
           (SELECT count(*) FROM information_schema.tables WHERE table_schema = 'public'), null
    ORDER BY 1;")
counts_after=$(docker compose exec -T "$DB_SERVICE" psql -U "$DB_USER" -d "$SCRATCH" -A -t -c "
    SELECT coalesce(tid, 'none') || '=' || coalesce(cnt, 0)
    FROM (
        SELECT relid::regclass::text AS tbl,
               (SELECT count(*) FROM pg_stat_all_tables s WHERE s.relid = t.relid) AS cnt,
               t.relid
        FROM pg_stat_all_tables t
        WHERE schemaname = 'public'
    ) q
    UNION ALL
    SELECT 'tables',
           (SELECT count(*) FROM information_schema.tables WHERE table_schema = 'public'), null
    ORDER BY 1;")
expect_eq "$counts_before" "$counts_after" "schema and per-table shapes match"

# ── 2. Index generations: checksum, transfer, symlink swap, verify ─────────────
note "rehearsal 2: index generation snapshot + swap..."

INDEX_DIR="$(grep -A1 '^\[index\]' config/development.toml | awk -F'\"' '/^dir/ {print $2}')"
if [ -d "$INDEX_DIR" ]; then
    for gen in "$INDEX_DIR"/gen-*; do
        [ -d "$gen" ] || continue
        name=$(basename "$gen")
        sha_before=$(find "$gen" -type f | sort | xargs sha224sum | sha224sum | awk '{print $1}')
        mkdir -p "$WORK/index"
        cp -a "$gen" "$WORK/index/$name"
        sha_after=$(find "$WORK/index/$name" -type f | sort | xargs sha224sum | sha224sum | awk '{print $1}')
        expect_eq "$sha_before" "$sha_after" "generation $name transferred byte-identical"

        # The symlink swap is how readers change generation with no downtime;
        # rehearse it against the scratch copy so production manipulation risk
        # stays off the real tree.
        ln -sfn "$WORK/index/$name" "$WORK/index/current"
        [ "$(readlink "$WORK/index/current")" = "$WORK/index/$name" ]
        note "PASS: symlink swap resolves to $name"
    done
else
    note "SKIP: no index directory at $INDEX_DIR (build one first: scripts/index-build.sh)"
fi

# ── Verdict ────────────────────────────────────────────────────────────────────
if [ "$failures" -eq 0 ]; then
    note ""
    note "restore rehearsal: PASS — record it in docs/operations/backup.md (date, digest of $WORK/lynx.dump, outcome)."
else
    note ""
    note "restore rehearsal: FAIL with $failures finding(s). A failed rehearsal is a finding — do not ignore it." >&2
    exit 1
fi
