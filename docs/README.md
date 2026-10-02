# Documentation — LYNX

Start at [`architecture.md`](./architecture.md). The complete specification is
[`architecture/specification.md`](./architecture/specification.md).

| Area | Documents |
| ---- | --------- |
| Getting started | [getting-started.md](./getting-started.md) · [DEVELOPMENT.md](../DEVELOPMENT.md) |
| Architecture | [architecture.md](./architecture.md) · [architecture/overview.md](./architecture/overview.md) · [architecture/specification.md](./architecture/specification.md) |
| Crawler | [crawler/](./crawler/README.md) — frontier, robots, downloader, parser, safety, politeness |
| Indexing | [indexing/](./indexing/README.md) — pipeline, tokenization, schema, dedup |
| Ranking | [ranking/](./ranking/README.md) — BM25F, freshness, authority, quality, spam, explainability |
| Search | [search/](./search/README.md) — query language, operators, pipeline, retrieval, pagination |
| Privacy | [privacy/](./privacy/README.md) — model, inventory, retention, telemetry, limitations, legal |
| Security | [security/](./security/README.md) — architecture, SSRF, auth, secrets, supply chain, incident response |
| API | [api/](./api/README.md) — REST, errors, rate limits, OpenAPI, versioning · [API.md](../API.md) |
| AI (Astra) | [ai/](./ai/README.md) — grounding, prompt injection, hallucination control |
| Operations | [operations/](./operations/README.md) — observability, runbooks, backups, DR, capacity |
| Database | [database/](./database/README.md) — ERD, schema notes, migrations, partitioning |
| Testing | [testing/](./testing/README.md) — strategy, quality evaluation |
| Decisions | [adr/](./adr/README.md) — architecture decision records |
| Diagrams | [diagrams/](./diagrams/README.md) |

Root documents: [README](../README.md) · [ARCHITECTURE](../docs/architecture.md) ·
[PRIVACY](../PRIVACY.md) · [SECURITY](../SECURITY.md) · [THREAT_MODEL](../docs/threat-model.md) ·
[API](../API.md) · [DEVELOPMENT](../DEVELOPMENT.md) · [DEPLOYMENT](../DEPLOYMENT.md) ·
[ROADMAP](../ROADMAP.md) · [CHANGELOG](../CHANGELOG.md) · [CONTRIBUTING](../CONTRIBUTING.md)

## Documentation rules

1. **Every claim is checkable.** Performance numbers come from a recorded run, security
   claims name their control, privacy claims name their mechanism. No marketing adjectives.
2. **Decisions are recorded, not re-litigated.** If you want to change an architecture
   choice, write an ADR that supersedes the old one; do not silently edit the old ADR.
3. **Diagrams are Mermaid** in Markdown, so they diff, review, and render in CI.
4. **No secrets, no real user data, no unlicensed content** in any document or fixture.
5. **Link, do not repeat.** One canonical location per fact; link to it.
6. **A doc that is wrong is worse than a doc that is missing.** Every page carries a
   "last reviewed" marker and an owner.