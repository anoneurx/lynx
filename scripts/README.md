# `scripts/` — developer tooling

Automation that is not part of a test suite or a build.

| Script | Purpose |
| ------ | ------- |
| `bootstrap.sh` | one-command toolchain setup (rustup pinned toolchain, pnpm via corepack, sqlx-cli, cargo-nextest, cargo-fuzz) |
| `dev.sh` | start/stop/status the local stack with hot reload for the web app |
| `db-migrate.sh` | thin wrapper over `sqlx migrate` with status and checksum verification |
| `db-seed.sh` | idempotent local seeding |
| `index-build.sh` | build a local index from a seed set for development |
| `crawl-fixture.sh` | run the crawler against the local fixture origin server only |
| `check-links.sh` | markdown link checker across `docs/` and root docs |
| `check-privacy.sh` | grep-based guard for banned log/metric fields and query persistence |
| `check-config.sh` | validate every `config/*.toml` against its environment validator |
| `bench.sh` | run criterion benchmarks and append to the perf record |
| `seed-ranking-corpus.sh` | regenerate the ranking golden fixtures from the hand-written corpus |

## Rules

1. `set -euo pipefail` and shellcheck clean.
2. Destructive scripts require an explicit confirmation flag and refuse to run when
   `LYNX_ENV=production`.
3. Scripts never accept secrets as positional arguments (visible in `ps`). Secrets come
   from the environment or a file.
4. No script contacts the public internet without an explicit `--allow-network` flag that
   is off by default. This keeps "curl some URL" out of the default path.
5. Scripts are POSIX-ish bash; `shfmt` formatting is enforced in CI.

Entry points for humans: `make help`.