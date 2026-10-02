# Fuzzing

## Targets

| Target | Crate | Entry point | Corpus |
| ------ | ----- | ----------- | ------ |
| `parse_query` | `lynx-query` | `lynx_query::parse` | `fuzz/corpus/parse_query/` |
| `parse_html` | `lynx-html` | `lynx_html::parse_fragment` | `fuzz/corpus/parse_html/` |
| `deserialize_tantivy` | `lynx-index` | `tantivy::schema::Schema::deserialize` | `fuzz/corpus/deserialize_tantivy/` |
| `bm25_scoring` | `lynx-ranking` | `ranking::bm25_batch` | `fuzz/corpus/bm25_scoring/` |
| `crawl_url_parse` | `lynx-crawler` | `crawler::canonicalize_url` | `fuzz/corpus/crawl_url_parse/` |

## Setup

```bash
# Install
cargo install cargo-fuzz

# Add target (once)
cargo fuzz init
cargo fuzz add parse_query
```

## Writing a fuzz target

```rust
// fuzz/fuzz_targets/parse_query.rs
#![no_main]
use libfuzzer_sys::fuzz_target;
use lynx_query::parse;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = parse(s);
    }
});
```

## Running locally

```bash
# Single target, 5 min
cargo fuzz run parse_query -- -max_total_time=300

# With corpus directory
cargo fuzz run parse_query -- fuzz/corpus/parse_query/

# Minimize crashing input
cargo fuzz tmin parse_query crash-<hash> -- -max_total_time=60
```

## Corpus management

### Seeding from real traffic

```bash
# Extract queries from production logs (anonymized)
./scripts/extract_queries.sh prod-logs/ > fuzz/corpus/parse_query/seeds.txt

# Add to corpus
cargo fuzz corpus parse_query add fuzz/corpus/parse_query/seeds.txt
```

### Pruning

```bash
# Remove duplicates, minimize
cargo fuzz cmin parse_query fuzz/corpus/parse_query/ fuzz/corpus/parse_query_min/
```

## CI integration

```yaml
# .github/workflows/fuzzing.yml
on:
  schedule:
    - cron: '0 3 * * 0'  # Weekly Sunday 03:00
  workflow_dispatch:

jobs:
  fuzz:
    runs-on: ubuntu-latest
    timeout-minutes: 240
    strategy:
      matrix:
        target: [parse_query, parse_html, deserialize_tantivy, bm25_scoring, crawl_url_parse]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@nightly
        with:
          components: rust-src
      - name: Install cargo-fuzz
        run: cargo install cargo-fuzz
      - name: Run fuzzing
        run: |
          cargo fuzz run ${{ matrix.target }} -- \
            -max_total_time=7200 \
            -artifact_prefix=fuzz/artifacts/${{ matrix.target }}/ \
            -runs=0
      - name: Upload crashes
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: fuzz-crashes-${{ matrix.target }}
          path: fuzz/artifacts/${{ matrix.target }}/
```

## OSS-Fuzz integration

Project registered at `https://oss-fuzz.com/`. Config in `projects/lynx/`.

```dockerfile
# projects/lynx/Dockerfile
FROM gcr.io/oss-fuzz/base-clang
RUN apt-get update && apt-get install -y rust
WORKDIR /src/lynx
COPY . .
RUN cargo fuzz build -O
```

Build: `python3 infra/helper.py build_image lynx`
Test: `python3 infra/helper.py build_fuzzers lynx`
Run: `python3 infra/helper.py run_fuzzer lynx parse_query`

## Sanitizers

| Sanitizer | Enabled | Command |
| --------- | ------- | ------- |
| Address (ASan) | ✅ | `cargo fuzz run ... -- -fsanitize=address` |
| Memory (MSan) | ✅ (nightly) | `cargo fuzz run ... -- -fsanitize=memory` |
| Thread (TSan) | ✅ | `cargo fuzz run ... -- -fsanitize=thread` |
| Undefined (UBSan) | ✅ | `cargo fuzz run ... -- -fsanitize=undefined` |

CI runs ASan + UBSan. MSan/TSan weekly (slower).

## Crash triage

1. **Reproduce locally:** `cargo fuzz run <target> fuzz/artifacts/<target>/crash-<hash>`
2. **Minimize:** `cargo fuzz tmin <target> crash-<hash>`
3. **Debug:** `cargo fuzz run <target> -- -runs=1 fuzz/artifacts/<target>/crash-<hash> 2>&1 | rustfilt`
4. **Fix:** Add regression test in `tests/fuzz_regressions/`.
5. **Update corpus:** Add minimized crash to corpus.

## Coverage reporting

```bash
# Generate coverage from fuzzing
cargo fuzz coverage parse_query -- -max_total_time=60
grcov . -s . --binary-path ./fuzz/target/x86_64-unknown-linux-gnu/release/parse_query \
  -t html --branch --ignore-not-existing -o fuzz/coverage/parse_query/
```

## Regression tests from fuzzing

```rust
// tests/fuzz_regressions/parse_query.rs
#[test]
fn fuzz_crash_empty_utf8_sequence() {
    // Minimized from crash-abc123
    let input = "\u{FFFD}"; // replacement char
    assert!(parse(input).is_ok()); // Should not panic
}
```

## Budget

- **CI time:** ~4 h/week (5 targets × 45 min each)
- **Corpus storage:** ~2 GB (compressed)
- **Crash retention:** 90 days in GitHub Artifacts

## Prioritization

New fuzz targets added when:
1. New public parser/decoder added.
2. Security audit finds attack surface.
3. Bug found in production that fuzzing would have caught.