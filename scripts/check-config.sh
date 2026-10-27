#!/usr/bin/env sh
# Validate every config/*.toml against its environment validator (scripts/README.md).
#
# The `lynx-config` test suite validates development.toml as development,
# production.toml as production, and example.toml as its declared environment, on
# every test run. This wrapper runs that suite so broken deployment configuration
# fails CI before it fails a production start.

set -eu

cd "$(dirname "$0")/.."

export TMPDIR=${TMPDIR:-/tmp}
exec cargo test -p lynx-config --locked
