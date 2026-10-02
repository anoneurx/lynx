# LYNX — developer entry points.
#
# `make help` lists everything. Documentation for the workflow lives in
# DEVELOPMENT.md; the targets here are the canonical names.

SHELL := /usr/bin/env bash
.SHELLFLAGS := -eu -o pipefail -c
.DEFAULT_GOAL := help

COMPOSE ?= docker compose
CARGO   ?= cargo
PNPM    ?= pnpm
CONFIG  ?= config/development.toml

.PHONY: help
help: ## Show this help
	@grep -hE '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) \
	  | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-22s\033[0m %s\n", $$1, $$2}'

# ── Setup ───────────────────────────────────────────────────────────────────────
.PHONY: bootstrap
bootstrap: ## Install pinned toolchains (rustup, pnpm, cargo-nextest, sqlx-cli, fuzz)
	@scripts/bootstrap.sh

.PHONY: env
env: ## Create .env from .env.example if missing
	@[ -f .env ] || { cp .env.example .env && echo "created .env (edit it)"; } || true

# ── Local stack ─────────────────────────────────────────────────────────────────
.PHONY: up
up: env ## Start the local stack
	@$(COMPOSE) up -d
	@scripts/dev.sh status

.PHONY: down
down: ## Stop the local stack
	@$(COMPOSE) down

.PHONY: clean
clean: ## Stop the stack and delete local volumes
	@$(COMPOSE) down -v

.PHONY: logs
logs: ## Tail local logs
	@$(COMPOSE) logs -f --tail=100

# ── Build ───────────────────────────────────────────────────────────────────────
.PHONY: build
build: ## Build all Rust crates and web apps
	@$(CARGO) build --workspace --all-targets
	@$(PNPM) -r build

.PHONY: release-build
release-build: ## Reproducible release build
	@$(CARGO) build --workspace --release --locked

# ── Quality gates ───────────────────────────────────────────────────────────────
.PHONY: fmt
fmt: ## Format Rust and docs
	@$(CARGO) fmt --all
	@$(PNPM) format

.PHONY: lint
lint: ## Clippy + eslint + shellcheck
	@$(CARGO) clippy --workspace --all-targets -- -D warnings
	@$(PNPM) lint
	@shellcheck scripts/*.sh

.PHONY: typecheck
typecheck: ## cargo check + tsc
	@$(CARGO) check --workspace --all-targets
	@$(PNPM) typecheck

.PHONY: check-links
check-links: ## Validate internal markdown links
	@scripts/check-links.sh

.PHONY: check-config
check-config: ## Validate every config/*.toml against its environment validator
	@scripts/check-config.sh

.PHONY: check-privacy
check-privacy: ## Privacy guard: banned log/metric fields, no query persistence
	@scripts/check-privacy.sh

# ── Tests ───────────────────────────────────────────────────────────────────────
.PHONY: test
test: ## Fast tests: unit + integration
	@$(CARGO) nextest run --workspace
	@$(COMPOSE) up -d lynx-db lynx-cache
	@scripts/db-migrate.sh
	@$(CARGO) nextest run -p lynx-test-integration

.PHONY: test-e2e
test-e2e: ## Browser tests
	@$(PNPM) test:e2e

.PHONY: test-security
test-security: ## Adversarial suites (SSRF, injection, privacy invariants)
	@$(CARGO) nextest run -p lynx-test-security

.PHONY: test-ranking
test-ranking: ## Ranking golden + metric gates
	@$(CARGO) nextest run -p lynx-test-ranking
	@$(CARGO) run -p lynx-eval --release -- --config $(CONFIG) --gate

.PHONY: ranking-accept
ranking-accept: ## Accept reviewed ranking golden changes (deliberate)
	@echo "Review the golden diff before running this."
	@$(CARGO) run -p lynx-eval --release -- --config $(CONFIG) --accept

.PHONY: test-perf
test-perf: ## Performance budgets (not part of default CI)
	@scripts/bench.sh

.PHONY: fuzz
fuzz: ## Bounded fuzz run for parser, URL, and query targets
	@$(CARGO) fuzz run -- -max_total_time=60

.PHONY: miri
miri: ## Miri over unsafe-free core (nightly)
	@$(CARGO) +nightly miri test -p lynx-common -p lynx-security

# ── Database ────────────────────────────────────────────────────────────────────
.PHONY: migrate
migrate: ## Apply pending migrations
	@scripts/db-migrate.sh up

.PHONY: migrate-status
migrate-status: ## Show applied/pending migrations
	@scripts/db-migrate.sh status

.PHONY: migrate-new
migrate-new: ## Scaffold a migration: make migrate-new NAME=add_robots_policy
	@test -n "$(NAME)" || { echo "usage: make migrate-new NAME=..."; exit 1; }
	@scripts/db-migrate.sh new "$(NAME)"

.PHONY: db-reset
db-reset: ## Drop, recreate, migrate, seed (local only)
	@scripts/db-migrate.sh reset

.PHONY: db-seed
db-seed: ## Idempotent local seed data
	@scripts/db-seed.sh

.PHONY: db-shell
db-shell: ## psql into the local database
	@$(COMPOSE) exec lynx-db psql -U lynx -d lynx

# ── Index & crawl (local fixtures only) ─────────────────────────────────────────
.PHONY: index-build
index-build: ## Build a local index from the dev seed set
	@scripts/index-build.sh

.PHONY: crawl-fixture
crawl-fixture: ## Run the crawler against the local fixture origin server
	@scripts/crawl-fixture.sh

# ── Docs ────────────────────────────────────────────────────────────────────────
.PHONY: docs
docs: ## Serve docs locally
	@$(PNPM) docs:dev

# ── Housekeeping ────────────────────────────────────────────────────────────────
.PHONY: supply-scan
supply-scan: ## cargo-deny, cargo-audit, image scan
	@$(CARGO) deny check
	@$(CARGO) audit
	@trivy fs --severity HIGH,CRITICAL --exit-code 1 .

.PHONY: tidy
tidy: ## cargo fmt + machete + udeps-style check + pnpm prune
	@$(CARGO) fmt --all
	@$(CARGO) machete
	@$(PNPM) install --frozen-lockfile

.PHONY: verify
verify: lint typecheck check-links check-config check-privacy test test-security test-ranking ## Everything that gates a PR
	@echo "verify: OK"