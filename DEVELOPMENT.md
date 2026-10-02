# Development — LYNX

How to run LYNX locally. Architecture first: [docs/architecture.md](./docs/architecture.md).
Deployment: [DEPLOYMENT.md](./DEPLOYMENT.md).

> This repository is currently at the **design stage**. The structure, tooling, configs,
> and CI exist; the services themselves are being implemented against
> [docs/architecture/specification.md §Z](./docs/architecture/specification.md#z-recommended-v01-implementation-order).

## Prerequisites

| Tool | Version | Why |
| ---- | ------- | --- |
| Rust | 1.78.0 (pinned in `Cargo.toml`) | Backend, crawler, indexer, ranker |
| Node.js | 20.11 | Web and admin apps only |
| pnpm | 9.12.3 | Workspace manager |
| Docker + Compose | 24+ | Postgres, Redis, full local stack |
| PostgreSQL | 16 | Local, or via Compose |
| Rust tool extras | — | `cargo-nextest`, `sqlx-cli`, `cargo-fuzz`, `cargo-deny`, `cargo-audit` |

`scripts/bootstrap.sh` (or `make bootstrap`) installs the pinned toolchain. `corepack`
handles pnpm.

## First run

```bash
git clone https://github.com/anoneurx/lynx
cd lynx

make bootstrap          # pinned toolchains
make env                # .env from .env.example
make up                 # postgres, redis, api, crawler, indexer, web, admin, prometheus, grafana
make migrate            # schema
make db-seed            # seed domains, blocklists, fixtures
make index-build        # build a local index from the seed set
```

Open:

- <http://localhost:3000> — web UI
- <http://localhost:8080/api/v1/search?q=test> — API
- <http://localhost:9091> — admin console (`dev` / `dev-insecure-change-me` in dev only)
- <http://localhost:9095> — Prometheus · <http://localhost:9096> — Grafana

Tear down with `make down`, or `make clean` to also drop volumes.

## Everyday commands

```bash
make help               # every target with a description
make verify             # the full PR gate: lint, typecheck, links, config,
                         # privacy guard, tests, security suites, ranking gates
make test               # unit + integration
make test-e2e           # browser tests
make test-security      # adversarial suites
make test-ranking       # ranking goldens + metric gates
make ranking-accept     # accept reviewed golden changes (deliberate act)
make test-perf          # performance budgets
make fuzz               # bounded fuzz run
make supply-scan        # cargo-deny, cargo-audit, trivy
```

## Local configuration

Layered, lowest to highest precedence:

```text
built-in defaults
  < config/development.toml
    < .env   (gitignored; developer overrides)
      < LYNX_* environment variables
        < --config-override flags   (refused when LYNX_ENV=production)
```

`config/example.toml` documents every key. Unknown keys are a **startup error**, not a
warning — a typo that silently falls back to a default is how a crawler ends up doing
something it should not.

Never put a secret in a config file. Use `{ secret_ref = "lynx/development/<service>/<name>" }`
and supply the value through the environment or the secret manager.

Validate config changes with `make check-config`.

## Configuration categories

```text
crawler · index · ranking · database · cache · security · privacy ·
api · observability · ai · admin
```

Settings under `[privacy]` that weaken the privacy model are documented in
[PRIVACY.md §8](./PRIVACY.md#8-configuration-that-can-weaken-this-model) and require an ADR.

## Working with the index locally

The index is a file on disk. Common operations:

```bash
make index-build        # rebuild from the dev seed set (deterministic)
make crawl-fixture      # crawl the local fixture origin server only
make db-reset           # drop, recreate, migrate, seed
```

Local development never crawls the public internet. The fixture origin server
(`lynx-fixture-origin`) scripts hostile responses — redirects to private addresses,
decompression bombs, traps, slowloris — so crawler behaviour is testable hermetically.

## Tests

| Command | Scope | CI lane |
| ------- | ----- | ------- |
| `make test` | unit + integration | every push |
| `make test-security` | SSRF, injection, authz, privacy invariants | every push |
| `make test-ranking` | goldens + metric gates | PR to main |
| `make test-e2e` | browser | main + nightly |
| `make test-perf` | budgets | nightly + release |
| `make fuzz` | parser, URL, query, safety gate | nightly |

No test touches the public internet. Security controls are tested for the negative case —
a test that only proves the happy path does not test a control.

## Coding standards

Rust:

```text
cargo fmt (default rustfmt)
cargo clippy --workspace --all-targets -- -D warnings
workspace lints: unsafe_code = "forbid"; pedantic on; unwrap/expect/panic = warn
edition 2021, MSRV 1.78
no network I/O in packages/*; no unbounded allocation or loops on untrusted input
all time access through a Clock trait so freshness/retention logic is testable
```

TypeScript:

```text
strict mode, noUncheckedIndexedAccess, exactOptionalPropertyTypes
no default exports in modules, no `any` without a written justification
ESLint flat config; Prettier formatting
```

Docs:

```text
markdownlint + prettier; internal links must resolve; mermaid must render
one canonical location per fact; every claim checkable
```

## Adding a dependency

1. Prefer an existing dependency; prefer no dependency.
2. Check the licence against the project's allow-list (`cargo deny`, `pnpm audit`).
3. Check the advisory database and the crate's release history.
4. Any dependency touching the network, filesystem, or deserialisation of untrusted input
   needs hand review and a note in the PR.
5. Never add a dependency with runtime telemetry, a build script that fetches from the
   network, or a licence incompatible with AGPL-3.0.
6. `make supply-scan` must pass.

## Migrations

```bash
make migrate-new NAME=add_robots_policy   # scaffold
make migrate                             # apply
make migrate-status                      # applied vs pending
make db-reset                            # local reset + migrate + seed
```

Migrations are forward-only and immutable once merged. Every migration is expand-only in
the PR that introduces it; the contract step ships later. No lock longer than 5 s. Any new
table declares a retention class, and any new column is either in
[docs/privacy/data-inventory.md](./docs/privacy/data-inventory.md) or it does not merge.

## Contributing flow

```text
issue → branch → PR → make verify → review → main → staging → release tag
```

See [CONTRIBUTING.md](./CONTRIBUTING.md) for review requirements and
[docs/adr/README.md](./docs/adr/README.md) for architecture decisions.

## Troubleshooting

| Symptom | Likely cause | Fix |
| ------- | ------------ | --- |
| `unknown configuration key` | Typo in a config or env var | The error names the key; check spelling |
| `secret_ref could not be resolved` | Missing env var or secret | Set it in `.env` for dev; secret manager otherwise |
| API returns `INDEX_UNAVAILABLE` | No index built, or wrong `index.dir` | `make index-build` |
| API returns empty results | Index built but not queried correctly | Check `config/development.toml` ranking config; run `make test-ranking` |
| Admin unreachable | Admin binds to `127.0.0.1` in dev | Use <http://localhost:9091> via Compose |
| Crawler refuses everything | Seed scopes or robots | `config/seeds/development.toml`; the fixture server allows crawling |
| `make verify` fails on links | New doc has a dead relative link | `scripts/check-links.sh` |
| `check-privacy` fails | A banned field name in a log or metric | Rename the field; do not suppress the check |
| Port already in use | Another service | Change the port in `.env` and `docker-compose.yml` together |

More: [docs/operations/troubleshooting.md](./docs/operations/troubleshooting.md).