# Deployment

## Container images

| Image | Base | Built from |
| ----- | ---- | ---------- |
| `ghcr.io/lynx/api` | `gcr.io/distroless/cc-debian12` | `docker/api.Dockerfile` |
| `ghcr.io/lynx/crawler` | `gcr.io/distroless/cc-debian12` | `docker/crawler.Dockerfile` |
| `ghcr.io/lynx/indexer` | `gcr.io/distroless/cc-debian12` | `docker/indexer.Dockerfile` |

Images are multi-platform (`linux/amd64`, `linux/arm64`) and signed with **cosign**.

```bash
# Build locally
docker buildx build --platform linux/amd64,linux/arm64 \
  -t ghcr.io/lynx/api:dev -f docker/api.Dockerfile .
```

## Kubernetes (recommended)

Manifests live in `deploy/kubernetes/`. Apply with Kustomize:

```bash
kubectl apply -k deploy/kubernetes/overlays/production
```

### Components

| Resource | Purpose |
| -------- | ------- |
| `Deployment/lynx-api` | Stateless search API, HPA driven by `http_requests_per_second` |
| `Deployment/lynx-crawler` | Crawler workers, scaled by queue depth (`lynx_crawl_queue_depth`) |
| `Deployment/lynx-indexer` | Single-replica leader-elected index builder |
| `StatefulSet/lynx-postgres` | Patroni cluster, 3 nodes, WAL-G to S3 |
| `StatefulSet/lynx-redis` | Redis Cluster, 6 shards |
| `PersistentVolume/lynx-index` | ReadOnlyMany NFS/EFS for Tantivy segments |

### Zero-downtime rolling updates

1. **API:** `maxSurge: 25%`, `maxUnavailable: 0`. New pods start, pass `/healthz` (checks
   Tantivy reader reload), then old pods drain connections (30 s grace).
2. **Crawler:** `maxUnavailable: 1`. In-flight fetches finish (SIGTERM → 60 s grace).
3. **Indexer:** Leader election via Postgres advisory lock. New pod becomes leader,
   builds generation *N+1*, publishes atomically (rename directory), signals readers
   via Redis pub/sub `INDEX_RELOAD`.

### Canary

```yaml
# deploy/kubernetes/overlays/canary/kustomization.yaml
patchesStrategicMerge:
  - canary-api.yaml   # 10 % weight via Ingress annotation
```

Promote after 10 min if `p99_latency < 200 ms` and `error_rate < 0.1 %`.

## Docker Compose (dev / single-node)

```bash
docker compose -f deploy/compose.yaml up -d
```

Brings up: `api`, `crawler`, `indexer`, `postgres`, `redis`, `minio` (S3 mock),
`grafana`, `prometheus`, `jaeger`.

## Environment variables (subset)

| Var | Required | Default | Description |
| --- | -------- | ------- | ----------- |
| `LYNX_DATABASE_URL` | yes | — | Postgres DSN |
| `LYNX_REDIS_URL` | yes | — | Redis DSN |
| `LYNX_INDEX_PATH` | yes | `/var/lib/lynx/index` | Tantivy segment directory (ReadOnlyMany) |
| `LYNX_BIND_ADDR` | no | `0.0.0.0:8080` | API listen address |
| `LYNX_LOG_LEVEL` | no | `info` | `trace`\|`debug`\|`info`\|`warn`\|`error` |
| `LYNX_TELEMETRY_ENDPOINT` | no | — | OTLP gRPC collector |

Full list: [`config.schema.json`](../configuration.md#schema).

## Secrets

All secrets injected via **ExternalSecrets** operator from Vault/AWS Secrets Manager.
See [secrets-management.md](./secrets-management.md).

## Health checks

| Endpoint | Probe | Timeout |
| -------- | ----- | ------- |
| `GET /healthz` | liveness | 2 s |
| `GET /readyz` | readiness | 5 s (checks Tantivy reader + Redis + Postgres) |

## Rollback

```bash
kubectl rollout undo deployment/lynx-api -n lynx
# Index rollback: rename previous generation directory, publish INDEX_RELOAD
```