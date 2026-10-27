# Telemetry

What leaves the process, when, and for how long. This page is the privacy-facing
view of [observability](../operations/observability.md); the mechanics there, the
privacy analysis here.

## What telemetry exists

| Kind | Content | Labels/fields | Lifetime | Where |
| ---- | ------- | ------------- | -------- | ----- |
| Request metrics | counts and duration histograms per route and outcome class | `service`, `endpoint`, `method`, `status_class`, `kind` | Prometheus series retention | `/metrics` on the admin port |
| Status pulse | the aggregate health surface in `/api/v1/status` | aggregate ratios only | runtime | response body |
| Structured logs | shape-only events: class codes, durations, counts | named fields from the permitted set | **14 days**, then gone | log pipeline |
| Distributed traces | head-sampled spans carrying method + path, request line stripped at emission | no client identifiers | collector default | OTLP endpoint (off in the base config) |
| Rate limiting | daily-rotating HMAC of the client network prefix, in memory | nothing durable is stored | process lifetime | RAM only |

## What telemetry cannot exist

- **Query text as a metric label** — banned field names fail CI
  (`scripts/check-privacy.sh`).
- **Query text as a log field** — the logging crate cannot serialise one
  (`packages/logging/src/shape.rs`), and the pipeline drops any banned-named field
  before persistence (`infrastructure/monitoring/log-pipeline.yml`).
- **Per-client anything** — the limiter's in-memory salted-prefix bucket is the
  closest thing to per-client state in the system, and it evaporates on restart
  without ever being written down.
- **Per-cohort compilation** — the visitor is one salted prefix among many at app
  startup, with no history to differ from (see
  [data-minimisation](./data-minimisation.md)).

## Config surfaces (the same knobs the validator polices)

| Knob | Invariant |
| ---- | --------- |
| `privacy.query_logging` | false in **every** environment; production start refuses otherwise |
| `observability.tracing` + `tracing_endpoint` | tracing on requires an endpoint (validator rule) |
| `observability.tracing_sample_ratio` | `[0, 1]` everywhere; **≤ 0.01 in production** — traces carry the request line, so higher head-sampling ratios are a leak vector |
| `observability.log_retention_days` | **≤ 14 in production** — nothing in the logs is worth archiving that is not in Postgres |
| `core.log_format` | `json` in production; pretty logs are refused |

## Operational consequence

Because telemetry carries no queries and no client identity, rebuilding every
dashboard, log search, and alert from a zero-data assumption is possible at any
time. Operators watch *shapes and aggregates* — a spike in a failing class, not a
trajectory of a person. If an incident response asks "what did this visitor
search?", the honest answer is "nothing in the system records that," and that is
the design, not an oversight.
