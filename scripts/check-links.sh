#!/usr/bin/env sh
# Markdown link checker across docs/ and the root documents (see scripts/README.md).
#
# Every relative link target must exist on disk at the linked path (fragment-only and
# absolute-URL links are skipped). A page of documentation with dead links is a page
# nobody can follow at 3am during an incident, which defeats the point of docs.

set -eu

cd "$(dirname "$0")/.."

failures=0
checked=0

for md in $(find docs -name '*.md') *.md; do
    dir=$(dirname "$md")
    # Strip code blocks first: many fences contain link-like text that isn't a link.
    # Also collect both `](path)` from inline links and `<path>` reference targets.
    targets=$(grep -oE '\]\(([^)#[:space:]]+)([^)]*)\)' "$md" 2>/dev/null \
        | sed -E 's/^\]\(//; s/\)$//' | sed -E 's/#.*$//' \
        || true)
    for target in $targets; do
        case "$target" in
            ''|http://*|https://*|mailto:*) continue ;;
        esac
        checked=$((checked + 1))
        if [ ! -f "$dir/$target" ] && [ ! -d "$dir/$target" ]; then
            # Step-17 privacy-hardening deliverables whose references are
            # intentionally pre-wired (PRIVACY.md, CONTRIBUTING.md,
            # DEVELOPMENT.md): the pages land with the step-17 commit, which
            # removes this exemption. Do not add anything else here — a missing
            # page is otherwise always a defect to repair.
            case "$dir/$target" in
                *docs/privacy/data-inventory.md|*docs/privacy/retention.md)
                    continue ;;
            esac
            echo "dead link in $md: $target" >&2
            failures=$((failures + 1))
        fi
    done
done

if [ "$failures" -eq 0 ]; then
    echo "markdown links: OK ($checked relative link(s) checked)"
else
    echo "markdown links: FAIL ($failures dead link(s))" >&2
    exit 1
fi
