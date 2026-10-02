# Getting Started

Run LYNX locally in 5 minutes.

## Prerequisites

| Tool | Version | Install |
| ---- | ------- | ------- |
| Rust | 1.78+ | `rustup` |
| Docker | 24+ | `docker.com` |
| Docker Compose | 2.20+ | included |
| `cargo-nextest` | latest | `cargo install cargo-nextest` |
| `cargo-llvm-cov` | latest | `cargo install cargo-llvm-cov` |

## Quick Start (Docker Compose)

```bash
git clone https://github.com/lynx/lynx
cd lynx

# Start all services (API, crawler, indexer, Postgres, Redis, MinIO, Grafana, Prometheus, Jaeger)
docker compose -f deploy/compose.yaml up -d

# Wait for health
curl -f http://localhost:8080/healthz
# {"status":"ok"}

# Search!
curl "http://localhost:8080/v1/search?q=rust+programming" | jq .
```

**Access:**
- API: `http://localhost:8080`
- Grafana: `http://localhost:3000` (admin/admin)
- Prometheus: `http://localhost:9090`
- Jaeger: `http://localhost:16686`
- MinIO Console: `http://localhost:9001` (minioadmin/minioadmin)

## Quick Start (Local Rust)

```bash
# 1. Start dependencies only
docker compose -f deploy/compose.yaml up -d postgres redis minio

# 2. Run migrations
sqlx migrate run --source migrations

# 3. Build & run API
cargo run --bin lynx-api

# 4. In another terminal: run crawler
cargo run --bin lynx-crawler

# 5. In another terminal: run indexer (one-time)
cargo run --bin lynx-indexer
```

## Configuration

Copy `config/local.toml.example` → `config/local.toml` and edit:

```toml
# config/local.toml
[server]
bind_addr = "0.0.0.0:8080"

[database]
url = "postgres://lynx:lynx@localhost:5432/lynx"

[redis]
url = "redis://localhost:6379"

[index]
path = "/tmp/lynx-index"

[logging]
level = "debug"
```

## Run Tests

```bash
# Unit + integration (fast, ~2 min)
cargo nextest run --workspace --all-targets

# With coverage
cargo llvm-cov --workspace --all-targets --html

# E2E (needs Docker, ~5 min)
cargo nextest run --test e2e -- --ignored
```

## Seed Data (Optional)

```bash
# Load 10k test documents
./scripts/seed_data.sh

# Or manually:
curl -X POST http://localhost:8080/v1/admin/crawl \
  -H "Content-Type: application/json" \
  -d '{"urls": ["https://rust-lang.org", "https://tokio.rs"]}'
```

## Project Structure

```
lynx/
├── crates/
│   ├── lynx-api/         # HTTP API (axum)
│   ├── lynx-crawler/     # Web crawler
│   ├── lynx-indexer/     # Tantivy index builder
│   ├── lynx-query/       # Query parsing
│   ├── lynx-ranking/     # BM25 + LTR
│   ├── lynx-html/        # HTML parsing
│   └── lynx-config/      # Configuration
├── deploy/
│   ├── compose.yaml      # Docker Compose
│   └── kubernetes/       # K8s manifests (Kustomize)
├── docs/                 # This documentation
├── migrations/           # SQLx migrations
├── scripts/              # Operational scripts
└── tests/                # Integration/E2E tests
```

## Next Steps

- [API Reference](../api/README.md)
- [Architecture Overview](../operations/README.md#architecture-in-production)
- [Configuration Guide](../configuration.md)
- [Contributing](../CONTRIBUTING.md)