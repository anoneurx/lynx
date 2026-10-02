# Configuration Reference

Complete configuration schema for LYNX services.

## File Locations

| Environment | Files (in order) |
| ----------- | ---------------- |
| Development | `config/base.toml` → `config/local.toml` → `LYNX_*` env vars |
| Staging | `config/base.toml` → `config/staging.toml` → `LYNX_*` env vars |
| Production | `config/base.toml` → `config/production.toml` → `LYNX_*` env vars |

**Secrets** (DB password, Redis password, S3 keys, TLS certs) are **never** in TOML.
Injected via Vault → ExternalSecrets → Kubernetes env vars → `LYNX_*` prefix.

## Schema

Full JSON Schema: [`config/schema.json`](../config/schema.json)

```bash
# Validate local config
cargo run --bin validate_config -- config/local.toml
```

## Server (`[server]`)

| Key | Type | Default | Description |
| --- | ---- | ------- | ----------- |
| `bind_addr` | string | `"0.0.0.0:8080"` | Listen address |
| `workers` | integer | `0` | Worker threads (0 = num CPUs) |
| `request_timeout_ms` | integer | `30000` | Request timeout |
| `max_request_size_mb` | integer | `10` | Max request body |
| `tls` | table | `null` | TLS config (see below) |

### TLS (`[server.tls]`)

| Key | Type | Default | Description |
| --- | ---- | ------- | ----------- |
| `cert_path` | string | — | Path to cert.pem |
| `key_path` | string | — | Path to key.pem |
| `ca_path` | string | — | Path to ca.pem (for mTLS) |

**In production**: certs injected via cert-manager → file mounts → paths in this config.

## Database (`[database]`)

| Key | Type | Default | Description |
| --- | ---- | ------- | ----------- |
| `url` | string | **required** | Postgres DSN (from Vault) |
| `max_connections` | integer | `100` | Pool max size |
| `min_connections` | integer | `10` | Pool min size |
| `connect_timeout_ms` | integer | `5000` | Connection timeout |
| `statement_timeout_ms` | integer | `30000` | Query timeout |
| `idle_timeout_ms` | integer | `600000` | Idle connection timeout |

**DSN format**: `postgres://user:pass@host:port/db?sslmode=require`

## Redis (`[redis]`)

| Key | Type | Default | Description |
| --- | ---- | ------- | ----------- |
| `url` | string | **required** | Redis DSN (from Vault) |
| `pool_size` | integer | `50` | Connection pool per worker |
| `timeout_ms` | integer | `1000` | Command timeout |
| `cluster_mode` | boolean | `true` | Enable Redis Cluster |

**DSN format**: `redis://:pass@host:port/0` or `redis-cluster://host:port`

## Index (`[index]`)

| Key | Type | Default | Description |
| --- | ---- | ------- | ----------- |
| `path` | string | `"/var/lib/lynx/index"` | Tantivy segment directory (ReadOnlyMany) |
| `generations_to_keep` | integer | `5` | Old generations to retain |
| `reload_signal_channel` | string | `"INDEX_RELOAD"` | Redis pub/sub channel |
| `reader_reload_timeout_ms` | integer | `5000` | Max wait for reader reload |

## Crawler (`[crawler]`)

| Key | Type | Default | Description |
| --- | ---- | ------- | ----------- |
| `concurrency` | integer | `64` | Concurrent fetches per worker |
| `user_agent` | string | `"LYNX/1.0 (+https://lynx.example.com/bot)"` | HTTP User-Agent |
| `request_timeout_ms` | integer | `10000` | Fetch timeout |
| `max_redirects` | integer | `5` | Max redirect hops |
| `max_response_size_mb` | integer | `50` | Max response body |
| `politeness` | table | see below | Per-domain rate limiting |

### Politeness (`[crawler.politeness]`)

| Key | Type | Default | Description |
| --- | ---- | ------- | ----------- |
| `default_rate_per_sec` | float | `0.5` | Requests/sec per domain (1/2s) |
| `burst` | integer | `1` | Token bucket burst |
| `respect_robots_txt` | boolean | `true` | Check robots.txt |
| `custom_rules` | table | `{}` | Domain-specific overrides |

```toml
[crawler.politeness.custom_rules]
"github.com" = { rate_per_sec = 1.0, burst = 2 }
"api.github.com" = { rate_per_sec = 5.0, burst = 10 }
```

## Indexer (`[indexer]`)

| Key | Type | Default | Description |
| --- | ---- | ------- | ----------- |
| `threads` | integer | `0` | Writer threads (0 = num CPUs) |
| `merge_policy` | string | `"log"` | `"log" \| "no_merge" \| "tiered"` |
| `commit_interval_docs` | integer | `10000` | Periodic commit frequency |
| `heap_size_mb` | integer | `1024` | Tantivy heap budget |

## Ranking (`[ranking]`)

| Key | Type | Default | Description |
| --- | ---- | ------- | ----------- |
| `first_pass_limit` | integer | `1000` | BM25 candidate count |
| `rerank_limit` | integer | `100` | LTR rerank count |
| `bm25_weight` | float | `0.3` | BM25 score weight |
| `ltr_weight` | float | `0.7` | LTR score weight |
| `model_path` | string | `"/models/lambdamart.bin"` | LambdaMART model file |

## Logging (`[logging]`)

| Key | Type | Default | Description |
| --- | ---- | ------- | ----------- |
| `level` | string | `"info"` | `trace\|debug\|info\|warn\|error` |
| `format` | string | `"json"` | `"json\|pretty"` |
| `include_trace` | boolean | `true` | Include trace_id/span_id |

## Telemetry (`[telemetry]`)

| Key | Type | Default | Description |
| --- | ---- | ------- | ----------- |
| `endpoint` | string | `""` | OTLP gRPC collector (empty = stdout) |
| `service_name` | string | `"lynx-api"` | OTel service name |
| `sample_rate` | float | `0.1` | Trace sampling (0.0–1.0) |
| `metrics_port` | integer | `9090` | Prometheus exporter port |

## Environment Variables

All config keys map to `LYNX_<SECTION>__<KEY>` (double underscore).

| Env Var | Config Path |
| ------- | ----------- |
| `LYNX_SERVER__BIND_ADDR` | `server.bind_addr` |
| `LYNX_DATABASE__URL` | `database.url` |
| `LYNX_REDIS__URL` | `redis.url` |
| `LYNX_INDEX__PATH` | `index.path` |
| `LYNX_CRAWLER__CONCURRENCY` | `crawler.concurrency` |
| `LYNX_LOGGING__LEVEL` | `logging.level` |

## Hot Reload (SIGHUP)

Send `SIGHUP` to reload **only these fields**:

- `logging.level`
- `crawler.concurrency`
- `crawler.politeness.*`
- `ranking.bm25_weight`, `ranking.ltr_weight`
- `telemetry.sample_rate`

```bash
# In container
kill -HUP 1

# In Kubernetes
kubectl exec -n lynx deploy/lynx-api -- kill -HUP 1
```

## Validation

```bash
# Generate schema
cargo run --bin generate_schema > config/schema.json

# Validate file
cargo run --bin validate_config -- config/production.toml

# CI check
cargo run --bin validate_config -- config/staging.toml config/production.toml
```

## Example: Production Override

```toml
# config/production.toml
[server]
workers = 32
request_timeout_ms = 15000

[database]
max_connections = 200
min_connections = 50

[redis]
pool_size = 100

[index]
generations_to_keep = 10

[crawler]
concurrency = 128
politeness.default_rate_per_sec = 1.0

[indexer]
threads = 32
merge_policy = "log"
heap_size_mb = 4096

[ranking]
first_pass_limit = 2000
rerank_limit = 200
model_path = "/models/lambdamart_v3.bin"

[telemetry]
endpoint = "otel-collector.lynx.svc:4317"
sample_rate = 0.05
```