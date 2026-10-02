# `database/schemas` — schema contract

The authoritative human-readable description of every table. The ERD lives at
[docs/database/erd.md](../../docs/database/erd.md); this directory is the per-table detail
that a diagram cannot carry.

```text
database/schemas/
├── README.md              this file
├── conventions.md         naming, id types, time types, null policy, enum policy
├── retention-classes.md   which retention class each table belongs to
└── tables/                one file per domain area
    ├── identity.md        admin_user, admin_role, api_key
    ├── crawl.md           domain, host, url, crawl_frontier, crawl_budget, crawl_ban
    ├── content.md         document, document_version, link, robots_policy
    ├── index_ops.md       index_manifest, index_shard, index_segment, dedup_cluster
    ├── governance.md      abuse_report, takedown_request, admin_audit_log
    └── quality.md         quality_evaluation_run, relevance_judgement, ranking_config_version
```

## Conventions

| Concern | Rule |
| ------- | ---- |
| Primary key | `uuid` v7, named `id`, never reused |
| Timestamps | `timestamptz` always. `created_at`/`updated_at` present on every mutable table |
| Soft delete | Only where history matters (`url`, `document`, `abuse_report`). Hard delete elsewhere |
| Enums | Postgres `enum` types for closed sets (`crawl_state`, `error_class`, `content_type`), with an added value migration required to extend |
| Money/licensing | Not applicable in v0.1 |
| Null | `NOT NULL` by default; null means "unknown" and is used sparingly and deliberately |
| Free text | Searchable text columns are documented with their language and encoding |
| PII | Any column holding personal data names its retention class and encryption method here, and exists in the data inventory |

## Retention classes

Every table declares one:

| Class | Meaning | Default retention |
| ----- | ------- | ----------------- |
| `crawl_permanent` | Life of the URL in the index | Until deindex/purge |
| `crawl_temporary` | Operational history | 30–90 days |
| `governance` | Legal/anti-abuse records | 90 days to 2 years |
| `aggregate` | Metrics rollups | 13 months |
| `no_user_data` | Contains no personal data by construction | n/a |

A table that cannot be assigned a class does not get created.

## Review requirement

Any PR touching this directory must also touch the ERD or explain why the ERD is
unchanged, and must keep [docs/privacy/data-inventory.md](../../docs/privacy/data-inventory.md)
in sync if it introduces or removes a column.