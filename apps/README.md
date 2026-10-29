# `apps/` — deployable applications

Everything in `apps/` produces a **running artifact**. Everything in `services/` is a
library or worker library. Everything in `packages/` is shared library code with no
deployment of its own.

| App | Path | Runtime | Exposed to internet | Purpose |
| --- | ---- | ------- | ------------------- | ------- |
| Web UI | [`web/`](./web/) | Static assets (SSG + CSR islands) | Yes | The LYNX search experience |
| Search API | [`api/`](./api/) | Rust / axum binary | Yes | Serves `/api/v1/*`, the only public machine surface |
| Admin console | [`admin/`](./admin/) | Static assets behind auth | **No** | Operations dashboard. Never internet-exposed |

**In v0.1.0, only [`api/`](./api/) is a running artifact** — the search API and its
server-rendered HTML SERP. [`web/`](./web/) and [`admin/`](./admin/) are design-only for
now: their READMEs describe the intended design, and the table above states the target
shape, not what ships today. See the [README status table](../README.md#status).

## Rules

1. `apps/web` and `apps/admin` are static bundles. They hold no server-side session, no
   database credentials, and no secret. Anything secret is called through `apps/api`.
2. `apps/api` is stateless. All state lives in Postgres / index / Redis. It can be scaled
   horizontally with no coordination beyond connection pooling.
3. `apps/admin` is bundled separately and served from a different hostname
   (`admin.` internal) behind VPN + SSO. It is never a route on the public API binary —
   see [ADR-0006](../../docs/adr/0006-api-and-admin-surface.md).
4. No app may log a raw search query. See [PRIVACY.md §3](../../PRIVACY.md#3-what-happens-to-a-query).

## Dependency direction

```text
apps/web ──HTTP──▶ apps/api ──▶ services/{query,ranker,suggestions}
apps/admin ─HTTP─▶ apps/api (admin routes) ──▶ services/*
                            │
                            └──▶ packages/*  (models, config, logging, security)
```

No `apps/*` may depend on `services/crawler` internals. The query plane and the crawl
plane are separate trust zones — see [docs/architecture.md §1](../../docs/architecture.md#1-system-context).