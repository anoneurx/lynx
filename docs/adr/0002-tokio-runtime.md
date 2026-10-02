# ADR 0002: Tokio as Async Runtime

**Date:** 2024-01-15
**Status:** Accepted
**Deciders:** Tech Lead, Platform Team
**Tags:** async, runtime

## Context

Rust async ecosystem has multiple runtimes: tokio, async-std, smol, glommio.
All core services are async (API, crawler, indexer).

## Decision

**Standardize on Tokio** (multi-threaded, work-stealing scheduler).

## Consequences

### Positive
- **Largest ecosystem** → most crates assume tokio (tower, axum, sqlx, tonic, redis)
- **Proven at scale** (Discord, Cloudflare, Linkerd)
- **Rich tooling** (tokio-console, tracing, metrics)
- **Work-stealing** → good for mixed I/O + CPU (crawler fetch + parse)

### Negative
- **Heavier** than smol/glommio (~200 KB binary overhead)
- **Global runtime** → harder to isolate (mitigated: separate processes per service)

## Configuration

```rust
// All services
#[tokio::main(flavor = "multi_thread", worker_threads = num_cpus::get())]
async fn main() { ... }
```

- `worker_threads` = physical cores (not hyperthreads)
- `max_blocking_threads` = 512 (for blocking DNS, TLS)

## Alternatives Rejected

| Runtime | Reason |
| ------- | ------ |
| async-std | Smaller ecosystem; unmaintained |
| smol | Too minimal; no built-in timer/driver |
| glommio | io_uring only; Linux-specific; immature |

## Related

- ADR 0001: Rust Language
- ADR 0010: Crawler Architecture (tokio tasks per connection)