#!/usr/bin/env sh
# Guard for banned log and metric fields (docs/operations/monitoring.md, PRIVACY.md).
#
# A high-cardinality label sourced from user input is a privacy leak with a very short
# time constant: it lands queries, URLs, or client identity in a time series database
# kept far longer than any log. This script greps the observability surface for the
# banned names and fails CI on any occurrence.
#
# Banned as metric LABELS / structured-log FIELD NAMES:
#   query q term terms url doc_id document_id request_uri ip client_ip user_agent email
#
# Enforcement points:
#   1. infrastructure/monitoring — no banned name may appear at all (it is the
#      observable contract).
#   2. Rust sources — metric/tracing emission sites only (label()/field()/record()
#      style constructors with a banned literal name). Ordinary identifiers named
#      `url` in code that handles URLs are not a finding: the ban is about what
#      enters a series or a log line, not what the program calls a variable.

set -eu

cd "$(dirname "$0")/.."

failures=0

BANNED='(query|q|term|terms|url|doc_id|document_id|request_uri|ip|client_ip|user_agent|email)'

# ── 1. Observability configuration: zero occurrences ────────────────────────────
for file in $(find infrastructure/monitoring -type f \( -name '*.yml' -o -name '*.yaml' -o -name '*.json' \)); do
    hits=$(grep -E -i -n "\"?[[:space:]]*:?[[:space:]]*\"?$BANNED\"" "$file" 2>/dev/null || true)
    if [ -n "$hits" ]; then
        echo "banned label/field in $file:" >&2
        printf '%s\n' "$hits" >&2
        failures=$((failures + 1))
    fi
done

# ── 2. Metric/tracing emission sites in Rust sources ───────────────────────────
# Only name positions of label/field constructors are checked. \b keeps a word
# boundary so tantivy index-schema calls (`add_text_field(`, `get_field(`) —
# which name *index fields*, not observable series — never match.
LABEL_PATTERN='(^|[^a-zA-Z0-9_])(label|metric_label|record|tag|record_field|field_name)\(\s*"'$BANNED'"'
hits=$(grep -rE -n "$LABEL_PATTERN" packages apps services --include='*.rs' 2>/dev/null || true)
if [ -n "$hits" ]; then
    echo "banned metric/log field name in Rust sources:" >&2
    printf '%s\n' "$hits" >&2
    failures=$((failures + 1))
fi

if [ "$failures" -eq 0 ]; then
    echo "banned log/metric fields: OK"
else
    echo "banned log/metric fields: FAIL ($failures file(s) above)" >&2
    exit 1
fi
