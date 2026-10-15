# Quality (`0009_search_quality.sql`, `0010_ranking_config.sql`)

The evaluation harness and versioned ranking configuration. Nothing here holds a
query string.

## `quality_evaluation_run` (`0009`)

One row per evaluation of a ranking configuration against a query set.

| Column | Type | Notes |
| ------ | ---- | ----- |
| `id` | `uuid` PK | |
| `name` | `text` | human label |
| `index_generation` | `bigint` | what was evaluated |
| `ranking_config_version` | `text` | |
| `query_set` | `text` | corpus identifier, not queries |
| `ndcg_at_10`, `precision_at_10`, `recall`, `mrr` | `real` | metric gates |
| `duplicate_rate`, `spam_rate` | `real` | quality gates |
| `passed` | `boolean` | CI gate result |
| `started_at`, `finished_at`, `notes` | | |

## `relevance_judgement` (`0009`)

The reviewed golden set. `query_id` is an **opaque id** into the versioned
evaluation corpus; the query text lives only in that corpus, never here.

`id`, `run_id` (→ `quality_evaluation_run`, `SET NULL` on delete), `query_id`,
`doc_id`, `grade` (`0..4`, enforced by `CHECK`), `labeler`
(`rule`\|`human`\|`model`).

## `ranking_config_version` (`0010`)

Every score is reproducible from `(doc_id, index generation,
ranking_config_version)`. A weight change bumps the version.

| Column | Type | Notes |
| ------ | ---- | ----- |
| `id` | `uuid` PK | |
| `version` | `text` UNIQUE | e.g. `v0.1.0` |
| `config` | `jsonb` | weights, `k1`/`b`, half-lives, penalties |
| `notes`, `created_by`, `created_at` | | |
| `active` | `boolean` | exactly one (`ranking_config_active_uniq`) |

The seeded `v0.1.0` row mirrors the constants in `services/ranker/src/config.rs`;
if one changes, both change in the same reviewed commit.
