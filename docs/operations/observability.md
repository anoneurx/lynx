# Observability

What LYNX measures, what it refuses to measure, and why every metric and log field
earns its place. The enforcement lives in `scripts/check-privacy.sh` (banned fields
fail CI) and `packages/config` — see [monitoring.md](./monitoring.md) for the metric
surface and [telemetry](../privacy/telemetry.md) for the privacy analysis.

## The invariant

**A metric or log field sourced from user input is a privacy leak with a very short
time constant.** It lands queries, URLs, or client identity in a time-series
database kept far longer than any log. So cardinality is a privacy control, not just
a cost control, and the ban list is enforced in CI, not left to review.

## Permitted metric label names

| Label | Why it exists | Cardinality bound |
| ----- | ------------- | ----------------- |
| `service` | Which binary emitted the metric | static — the crate name |
| `endpoint` | Which HTTP surface (`/api/v1/search`, `/api/v1/suggestions`, …) | static — the route table |
| `method` | HTTP verb | 9 |
| `status_class` | `2xx/4xx/5xx` — never the raw status code, never the body | 3 |
| `kind` | Response classification (`hit`, `miss`, `error`, `denied`) | < 10 |
| `generation` | Index generation, only on index-plane gauges | bounded by retention |
| `host` / `plan` | Crawler-plane internals, keyed on **crawled** hosts, not users | host budget bound |
| `shard` | Index shard ordinal | configured, < 32 |

Everything else needs a written justification next to the metric definition and a
re-review of its cardinality contract.

## Banned field/label names (CI-enforced)

`query`, `q`, `term`, `terms`, `url`, `doc_id`, `document_id`, `request_uri`, `ip`,
`client_ip`, `user_agent`, `email` — as metric labels or structured-log field names,
in any observability configuration or emission site. The check greps
`infrastructure/monitoring` and Rust metric/tracing constructors.

Why each is banned, briefly: query and term variants put user intents on disk;
`url`/`doc_id` tie a network peer to pages they read; `ip`/`client_ip`/`user_agent`
are the identity LYNX *deliberately does not use* (see
[PRIVACY.md](../../PRIVACY.md)); `request_uri` smuggles the search string through a
path; `email` has no legitimate metric.

## Latency, sampled honestly

Request latency is recorded as a histogram with `endpoint` and `status_class` labels
only. The p50/p95/p99 reported in `/api/v1/status` come from `histogram_quantile` on
that histogram — there is no per-request timer map and no per-user anything.

## Distributed tracing (OTel)

| Knob | Where | Contract |
| ---- | ----- | -------- |
| `observability.tracing` | config | master switch; ships spans to OTLP |
| `observability.tracing_endpoint` | config | collector OTLP endpoint; required when tracing is on (validator refuses otherwise) |
| `observability.tracing_sample_ratio` | config | head-sampling ratio, `0…1` everywhere; **production is capped at 0.01** by the validator |

Span contract: the HTTP server span records `method` and the path **without the
query string** at emission (`apps/api/src/router.rs`), so no sampling ratio can leak
a search string, and the client IP never enters span attributes — the limiter's
salted HMAC of a network prefix is all a span could ever see.

## Logs

- Shape-only by construction: the logging crate cannot serialise a query — see
  `packages/logging/src/shape.rs`.
- JSON in production (`core.log_format`), pretty elsewhere.
- Pipeline-level redaction (`infrastructure/monitoring/log-pipeline.yml`): drop any
  field matching the ban list before persistence, strip everything after `?` in
  any path-like field, keep 14 days — then drop. No long-term log archive; anything
  worth keeping is in Postgres (see
  [backup.md](./backup.md) for what that means).

## Backups and restore

Backup/restore procedures and the standing rehearsal requirement live in
[backup.md](./backup.md); the rehearsal script is
`infrastructure/scripts/restore-rehearsal.sh`. A backup that has not been restored
is a hypothesis, not a backup.
