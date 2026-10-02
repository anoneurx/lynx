# `tests/integration` — cross-service integration

Tests that wire real components together against real dependencies (Postgres in a
testcontainer, a real Tantivy index in a temp directory). No mocks of our own types — if a
test mocks `SearchIndexReader`, it belongs in `tests/unit`.

| Suite | Chain |
| ----- | ----- |
| `crawl` | frontier → crawler → parser → indexer |
| `index` | parser output → indexer → index reader |
| `search` | query processor → index reader → ranker |
| `api` | HTTP request → middleware → API → index → response |
| `config` | every `config/*.toml` loads and validates in its environment |
| `migrations` | migrate from empty, from previous version, and roll-forward |

Rules: hermetic (temp dirs, ephemeral containers), deterministic (injected clock), and no
network access. `make test-integration` spins up dependencies once and reuses them.