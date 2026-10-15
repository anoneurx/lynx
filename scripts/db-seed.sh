#!/usr/bin/env bash
#
# Apply idempotent local seed data.
#
# Every seed statement uses ON CONFLICT DO NOTHING, so running this repeatedly is
# safe. It never touches real data: seeds are development and fixture rows only.

set -euo pipefail

readonly SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ $# -gt 0 ]]; then
  echo "usage: db-seed.sh   (DATABASE_URL comes from the environment)" >&2
  exit 2
fi

cd "${SELF_DIR}/.."
exec cargo run -q -p lynx-storage --bin lynx-db -- seed
