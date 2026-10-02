# `database/seeds` — development seed data

Idempotent seed data for local development, integration tests, and reproducible benchmark
runs.

```text
database/seeds/
├── README.md
├── domains.toml            allowed seed domains + per-domain crawl budget overrides
├── blocklist.txt           domains/hosts excluded from crawling (self-promo, personal, NSFW policy)
├── term_blocklist.txt      suggestion + ranking policy terms
├── sample_urls.txt         a small, stable, hand-reviewed URL set for integration tests
└── fixtures/               deterministic documents for ranking/search tests
```

## Rules

1. **No production content.** No crawled page bodies, no real third-party HTML, no
   user data, no API keys. Seeds are either configuration-like lists or synthetic fixtures.
2. **Idempotent.** `make db-seed` can be run repeatedly without duplicating rows.
3. **Deterministic.** Ranking and indexer tests read from `fixtures/`, so a golden-file
   test gives the same answer on every machine.
4. **Reviewed.** `blocklist.txt` is editorial content and goes through the same review as
   ranking config — see [docs/ranking/spam.md](../../docs/ranking/spam.md).
5. **No live-network tests from seeds.** Integration tests run against a local HTTP fixture
   server, never the public internet, so CI is hermetic and non-abusive.

## Local development seed set

The default development scope is deliberately tiny (~50 URLs across a handful of
hand-picked domains, including pages designed to exercise robots, redirects, canonical
tags, duplicates, and traps). It is not a crawl of the live web. CI must never fetch from
`sample_urls.txt`.