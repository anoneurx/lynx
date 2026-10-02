# ADR 0021: Configuration Management

**Date:** 2024-08-15
**Status:** Accepted
**Deciders:** Platform Team, Tech Lead
**Tags:** configuration, config, schema

## Context

Configuration needs:
- Type-safe, validated at startup
- Environment-specific (dev/staging/prod)
- Secrets separate (Vault)
- Hot-reload for non-secrets (log level, cache TTL)
- Documentation + schema for IDE support
- Single source of truth

Options: TOML + `config` crate, YAML + `serde_yaml`, JSON + `serde_json`, Figment, Confy.

## Decision

**TOML files + `config` crate + JSON Schema generation.**

### File Hierarchy

```
config/
├── base.toml           # Defaults (committed)
├── local.toml          # Dev overrides (gitignored)
├── staging.toml        # Staging overrides (committed)
├── production.toml     # Prod overrides (committed)
└── schema.json         # Generated JSON Schema (committed)
```

### Loading Order (precedence)

1. `base.toml` (defaults)
2. `{environment}.toml` (staging/production)
3. `local.toml` (dev only)
4. Environment variables (`LYNX_*` prefix)
5. Secrets (Vault → env vars via ExternalSecrets)

### Config Struct (Rust)

```rust
// src/config.rs
use config::{Config, Environment, File};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub index: IndexConfig,
    pub crawler: CrawlerConfig,
    pub ranking: RankingConfig,
    pub logging: LoggingConfig,
    pub telemetry: TelemetryConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    pub bind_addr: String,           // "0.0.0.0:8080"
    pub workers: usize,              // 0 = auto
    pub request_timeout_ms: u64,     // 30000
    pub max_request_size_mb: usize,  // 10
    pub tls: Option<TlsConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DatabaseConfig {
    pub url: String,                 // Postgres DSN (from Vault)
    pub max_connections: u32,        // 100
    pub min_connections: u32,        // 10
    pub connect_timeout_ms: u64,     // 5000
    pub statement_timeout_ms: u64,   // 30000
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IndexConfig {
    pub path: PathBuf,               // "/var/lib/lynx/index"
    pub generations_to_keep: usize,  // 5
    pub reload_signal_channel: String, // "INDEX_RELOAD"
    pub reader_reload_timeout_ms: u64, // 5000
}

// ... other configs

impl AppConfig {
    pub fn load() -> Result<Self> {
        let env = std::env::var("LYNX_ENV").unwrap_or_else(|_| "development".into());
        let mut builder = Config::builder()
            .add_source(File::with_name("config/base").required(true))
            .add_source(File::with_name(&format!("config/{env}")).required(false))
            .add_source(File::with_name("config/local").required(false))
            .add_source(Environment::with_prefix("LYNX").separator("__"));
        
        // Secrets injected as env vars by ExternalSecrets
        let config = builder.build()?;
        config.try_deserialize()
    }
}
```

### JSON Schema Generation

```bash
# At build time
cargo run --bin generate_schema > config/schema.json
```

```rust
// generate_schema.rs
use schemars::schema_for;
use lynx_config::AppConfig;

fn main() {
    let schema = schema_for!(AppConfig);
    println!("{}", serde_json::to_string_pretty(&schema).unwrap());
}
```

### IDE Integration

- **VS Code**: `yaml-language-server` + `config/schema.json` → autocomplete, validation
- **IntelliJ**: JSON Schema mapping → same benefits

### Hot Reload (SIGHUP)

```rust
// Only for non-secret, non-structural config
tokio::signal::unix::signal(SignalKind::hangup())?.recv().await;
let new_config = AppConfig::load()?;
app_state.update_config(new_config)?;  // Atomic swap Arc<Config>
```

**Reloadable fields**: `logging.level`, `crawler.concurrency`, `ranking.weights`, `cache.ttl`.
**NOT reloadable**: `database.url`, `server.bind_addr`, `index.path`.

### Secrets

- **Never in TOML** → `database.url`, `redis.password`, `s3.secret_key` from Vault
- Injected as env vars: `LYNX_DATABASE_URL`, `LYNX_REDIS_PASSWORD`
- `config` crate reads them via `Environment::with_prefix("LYNX")`

## Consequences

### Positive
- **Type-safe** → compile-time + runtime validation
- **Documented** → schema.json = living documentation
- **Environment parity** → same code, different config files
- **Hot reload** → no restart for tuning

### Negative
- **Two sources** (TOML + env vars) → precedence complexity
- **Schema generation** requires build step

## Alternatives Rejected

| Approach | Reason |
| -------- | ------ |
| YAML | No comments in JSON Schema; TOML more readable |
| Figment | Less common, similar complexity |
| Pure env vars | No defaults, no validation, no docs |
| Consul/etcd | Overkill; config changes rare |

## Related

- ADR 0006: Secrets Management (Vault)
- ADR 0009: Kubernetes (ExternalSecrets)
- docs/configuration.md (user-facing docs)