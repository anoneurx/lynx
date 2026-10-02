# ADR 0018: Fuzzing Strategy

**Date:** 2024-07-01
**Status:** Accepted
**Deciders:** Security Team, Platform Team
**Tags:** fuzzing, security, testing

## Context

Attack surface:
- Query parser (user input)
- HTML parser (untrusted web content)
- URL parser/canonicalizer
- Tantivy schema deserialization
- BM25 scoring (numeric edge cases)

Need continuous fuzzing to catch memory safety bugs, logic errors, DoS vectors.

## Decision

**Coverage-guided fuzzing with `cargo-fuzz` (libFuzzer) + OSS-Fuzz integration.**

### Targets

| Target | Crate | Entry Point | Priority |
| ------ | ----- | ----------- | -------- |
| `parse_query` | `lynx-query` | `lynx_query::parse` | Critical |
| `parse_html` | `lynx-html` | `lynx_html::parse_fragment` | Critical |
| `canonicalize_url` | `lynx-crawler` | `crawler::canonicalize_url` | High |
| `deserialize_schema` | `lynx-index` | `tantivy::schema::Schema::deserialize` | Medium |
| `bm25_batch` | `lynx-ranking` | `ranking::bm25_batch` | Medium |

### Fuzz Target Template

```rust
// fuzz/fuzz_targets/parse_query.rs
#![no_main]
use libfuzzer_sys::fuzz_target;
use lynx_query::parse;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Limit input size to prevent OOM in fuzzer
        if s.len() > 10_000 { return; }
        let _ = parse(s);
    }
});
```

### Corpus Management

1. **Seed from production** (anonymized):
   ```bash
   ./scripts/extract_queries.sh logs/ > fuzz/corpus/parse_query/seeds.txt
   cargo fuzz corpus parse_query add seeds.txt
   ```
2. **Minimize** after each run: `cargo fuzz cmin`
3. **Cross-pollinate**: Share corpora between similar targets

### CI Integration

```yaml
# .github/workflows/fuzzing.yml
on:
  schedule:
    - cron: '0 3 * * 0'  # Weekly
  workflow_dispatch:
jobs:
  fuzz:
    runs-on: ubuntu-latest
    timeout-minutes: 240
    strategy:
      matrix:
        target: [parse_query, parse_html, canonicalize_url, deserialize_schema, bm25_batch]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@nightly
      - run: cargo install cargo-fuzz
      - run: cargo fuzz run ${{ matrix.target }} -- -max_total_time=7200
```

### OSS-Fuzz

- Project registered: `https://oss-fuzz.com/project/lynx`
- Build: `python3 infra/helper.py build_fuzzers lynx`
- Continuous fuzzing on Google infrastructure (thousands of cores)
- Automatic crash reporting → GitHub Security Advisory

### Sanitizers

| Sanitizer | Targets | Frequency |
| --------- | ------- | --------- |
| Address (ASan) | All | Every run |
| Undefined (UBSan) | All | Every run |
| Memory (MSan) | All | Weekly (nightly) |
| Thread (TSan) | All | Weekly (nightly) |

### Crash Triage

1. **Minimize**: `cargo fuzz tmin <target> crash-<hash>`
2. **Reproduce**: `cargo fuzz run <target> crash-<hash> -- -runs=1`
3. **Fix**: Add regression test in `tests/fuzz_regressions/`
4. **Verify**: `cargo fuzz run <target>` passes

## Consequences

### Positive
- **Continuous** → catches regressions early
- **Real-world corpus** → finds bugs production inputs trigger
- **OSS-Fuzz** → massive compute free

### Negative
- **CI time** (~4h/week)
- **Maintenance** (corpus updates, new targets)
- **False positives** (crashes in test-only code paths)

## Alternatives Rejected

| Approach | Reason |
| -------- | ------ |
| Property-based testing only | Doesn't explore deep state spaces |
| Manual penetration testing | Not continuous, expensive |
| AFL++ only | `cargo-fuzz` wraps libFuzzer + AFL++ compatible |

## Related

- ADR 0017: Sandboxed HTML Parsing (fuzz target)
- ADR 0001: Rust (memory safety baseline)
- Security Runbook RB-09: Fuzzing Crash Response