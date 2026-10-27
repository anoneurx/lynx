# Data Inventory

Every place LYNX stores anything, whether it names a person, and the policy that
bounds it. This page is the completeness claim behind
[data-minimisation](./data-minimisation.md): "we store as little as possible" is
only worth what an itemised list is worth. Retention mechanics are in
[retention.md](./retention.md).

Boundary note first: **the search index (Tantivy) and Postgres hold different
things**, and conflating them is the classic mistake. The index holds the
corpus; Postgres holds operational state *for that corpus*; neither holds user
data, because no user identification exists (see
[PRIVACY.md](../../PRIVACY.md)).

## 1. The corpus (Tantivy index — never user data)

| Field in the index | What it contains | How it arrived | Class |
| ------------------ | ---------------- | -------------- | ----- |
| `doc_id` | UUIDv7 identifier derived from the canonical URL | assigned at index time | corpus |
| `url` | The canonical page URL | crawled and canonicalised | corpus |
| `urlhash` | BLAKE3 of the canonical URL — join key, non-reversible | derived | corpus |
| `title`, `description` | Headline and description as published | extracted from crawled HTML | corpus |
| `body` | The page's extracted *text* | parser output; **raw HTML is never stored** | corpus |
| anchor/industry/lang/published/modified fields | Structured extraction from the page | extracted | corpus |
| `quality_score` | LYNX's own quality estimate | computed on the index plane | corpus |
| `dedup` | Near-duplicate signature | computed at parse time | corpus |

Deindexing and takedowns remove entries with the **next generation** (confirmed
by that generation's checksum audit — the flow is in [legal](./legal.md)).

## 2. Postgres — operational state

Every table is classified in
[`services/storage/src/retention.rs`](../../services/storage/src/retention.rs),
which doubles as a build fence: a new table with no class is a build failure.

### `crawl_permanent` — life of the entry in the corpus

`domain`, `host`, `url`, `crawl_frontier`, `crawl_budget`, `document`,
`document_version`, `link`, `robots_policy`: the origins being indexed, their
frontier rows, the extracted document relation, and raw `robots.txt` kept for
compliance disputes. **No personal data** by construction: every row concerns an
origin or an extracted document. Rows leave when the corresponding URL leaves
the index (deindex or takedown).

### `crawl_temporary` / `aggregate` — bounded operational history

`crawl_attempt` (30-day diagnostic history — outcomes and error classes, no
content, no client data), `crawl_stat_daily` (per-host daily rollups). No
personal data: hosts here are *crawled origins*, not visitors.

### `governance` — never clock-swept

`admin_user` and roles (operator identities), `api_key` (owner **email** — one
of the two personal-data stores; defensively field-encrypted),
`abuse_report` (reporter contact — the second, likewise field-encrypted and
HMAC-deduped), `takedown_request` (complainant context), `admin_audit_log`
(append-only record of admin actions). Erasure here is a domain action on
request, never a bulk sweep — see [retention.md](./retention.md) for why.

### `no_user_data` — contains none by construction

`index_manifest`, `index_shard`, `index_segment`, `dedup_cluster`,
`quality_evaluation_run`, `relevance_judgement`, `ranking_config_version`,
`schema_migrations`, `retention_policy`.

## 3. In-memory state (process lifetime)

| State | Content | Lifetime |
| ----- | ------- | -------- |
| Rate-limit buckets | HMAC of the client /24 network prefix under a daily salt | evaporates at restart |
| Query response cache | truncated query hash → rendered response | cache TTL or eviction |
| Index reader handles | mmapped Tantivy segment files | until generation swap |

## 4. Observability surfaces

Per [telemetry.md](./telemetry.md): metrics restricted to the permitted label
set (`service`, `endpoint`, `method`, `status_class`, `kind` and friends), logs
that are shape-only and dropped after 14 hot days, traces head-sampled and
capped at 1% in production, and **never the query string** — the ban list is
CI-enforced (`scripts/check-privacy.sh`).

## 5. Backups

Postgres base backup + WAL-G, encrypted; index generations copied through the
same path with checksums; a 35-day ceiling overall. A backup inherits every
restriction in this inventory — a restore reconstructs the stored state at its
backup point, which is why the rotation of backups must not outlive the archive
promise for `governance` data documented in
[backup.md](../operations/backup.md).

## 6. What is deliberately absent

- No cookies, no accounts, no sessions, no client identifiers of any kind.
- No query string in any log, metric, trace, or table (PRIVACY.md §7).
- No per-IP history: the rate limiter's HMAC bucket is the only identity-
  adjacent state in the system, and it is not durable.
- No client-side storage or fingerprinting; the GPC and header tests pin the
  observable behaviour (see [telemetry.md](./telemetry.md)).
