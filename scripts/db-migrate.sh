#!/usr/bin/env bash
#
# Apply and inspect LYNX database migrations.
#
# This drives `lynx-db` (services/storage) rather than sqlx-cli, because the
# `schema_migrations` ledger the project documents has exactly one writer: the
# Rust migration runner. Two tools writing two ledgers is how environments
# diverge.
#
#   db-migrate.sh up       apply pending migrations
#   db-migrate.sh status   list applied and pending
#   db-migrate.sh new NAME scaffold the next NNNN_NAME.sql
#   db-migrate.sh reset    drop, recreate, migrate, seed (local only, destructive)
#
# DATABASE_URL is required for up/status/reset. Secrets come from the environment,
# never as arguments.

set -euo pipefail

readonly SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

usage() {
  sed -n '3,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

run_db() {
  ( cd "${SELF_DIR}/.." && cargo run -q -p lynx-storage --bin lynx-db -- "$@" )
}

command="${1:-up}"
case "${command}" in
  up | status)
    run_db "${command}"
    ;;
  new)
    if [[ $# -lt 2 || -z "${2:-}" ]]; then
      echo "usage: db-migrate.sh new NAME" >&2
      exit 2
    fi
    run_db new "${2}"
    ;;
  reset)
    : "${DATABASE_URL:?DATABASE_URL must be set}"
    if [[ "${LYNX_ENV:-development}" == "production" ]]; then
      echo "refusing to reset the schema with LYNX_ENV=production" >&2
      exit 1
    fi
    echo "dropping and recreating schema public on ${DATABASE_URL%%\?*}" >&2
    psql "${DATABASE_URL}" -v ON_ERROR_STOP=1 \
      -c 'DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public;'
    run_db up
    "${SELF_DIR}/db-seed.sh"
    ;;
  -h | --help | help)
    usage
    ;;
  *)
    echo "unknown command: ${command}" >&2
    usage >&2
    exit 2
    ;;
esac
