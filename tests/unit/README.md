# `tests/unit` — cross-crate unit tests

Single-crate unit tests normally live in the source file they cover. This directory holds
only the cases that would be awkward there: shared golden corpora, table-driven cases over
several modules at once, and tests that need a fixture file.

- `fixtures/` — checked-in, deterministic inputs (URL lists, HTML samples, query sets)
- `golden/` — expected outputs; diffs are reviewed
- Property tests (`proptest`) for URL normalisation, tokenisation, and query parsing

No I/O beyond fixture reads. Runs in under a second, in every CI lane.