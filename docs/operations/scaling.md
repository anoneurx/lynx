# Scaling

## Search API (lynx-api)

Stateless, horizontally scalable behind L7 load balancer.

| Metric | Scale-up threshold | Scale-down threshold |
| ------ | ------------------ | -------------------- |
| `http_requests_per_second` per pod | > 1,200 req/s | < 400 req/s |
| `p99_latency_ms` | > 250 ms | < 100 ms |
| `cpu_utilization` | > 70 % | < 30 % |

**HPA example:**

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: lynx-api
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: lynx-api
  minReplicas: 3
  maxReplicas: 50
  metrics:
    - type: Pods
      pods:
        metric:
          name: http_requests_per_second
        target:
          type: AverageValue
          averageValue: "1200"
  behavior:
    scaleDown:
      stabilizationWindowSeconds: 300
      policies:
        - type: Percent
          value: 10
          periodSeconds: 60
```

## Tantivy read replicas

Index segments stored on **ReadOnlyMany** volume (NFS/EFS/CephFS).
Add API pods freely; each opens its own `IndexReader`.
Reload triggered by Redis pub/sub `INDEX_RELOAD` (sub-millisecond propagation).

### Sharding (future)

For > 1 B documents: split index by hash(`url`) into **N** independent Tantivy indexes.
Router (API) forwards to shard `hash(url) % N`. Each shard has its own reader pool.

## Crawler workers (lynx-crawler)

Scale by queue depth metric `lynx_crawl_queue_depth` (RabbitMQ/Redis Streams).

| Queue depth per worker | Action |
| ---------------------- | ------ |
| > 5,000 URLs | scale up |
| < 500 URLs | scale down |

**Concurrency per worker:** `LYNX_CRAWLER_CONCURRENCY=64` (async I/O, ~2k sockets).
Max 200 workers before politeness/domain limits dominate.

### Politeness

Per-domain token bucket (1 req / 2 s default). Configured in `crawler.politeness` section
of config. Workers share state via Redis sorted sets.

## Indexer (lynx-indexer)

Single-leader (Patroni-style lock in Postgres). Builds generation *N+1* in background.
Build time scales linearly with corpus size:

| Documents | Build time (8 vCPU, NVMe) |
| --------- | ------------------------- |
| 10 M | ~12 min |
| 100 M | ~2 hr |
| 1 B | ~20 hr |

**Speed-up levers:**
- Increase `LYNX_INDEXER_THREADS` (default: physical cores)
- Use `--merge-policy=log` for faster merges
- Pre-sort input by URL hash (improves segment locality)

## Postgres (operational DB)

### Sizing

| Rows (pages) | Recommended instance | Storage |
| ------------ | -------------------- | ------- |
| < 10 M | `db.r6g.xlarge` (4 vCPU, 32 GiB) | 500 GiB gp3 |
| 10–100 M | `db.r6g.2xlarge` (8 vCPU, 64 GiB) | 2 TiB gp3 |
| 100 M–1 B | `db.r6g.4xlarge` (16 vCPU, 128 GiB) | 6 TiB gp3 + WAL-G to S3 |

### Read replicas

Add 2–3 read replicas for:
- Analytics dashboards
- Admin API (`/admin/stats`)
- Crawler scheduler lookups

Primary handles: crawl queue, indexer metadata, user accounts.

### Connection pooling

**PgBouncer** in transaction mode (`pool_mode = transaction`).
`max_client_conn = 10,000`, `default_pool_size = 200`.

## Redis (cache + coordination)

| Use case | Key pattern | TTL |
| -------- | ----------- | --- |
| Search result cache | `search:{hash(query,opts)}` | 5 min |
| Suggest prefix cache | `suggest:{prefix}` | 10 min |
| Crawl politeness tokens | `politeness:{domain}` | 2 s sliding |
| Index reload pub/sub | channel `INDEX_RELOAD` | — |

**Cluster mode:** 6 shards, 1 replica each. `maxmemory-policy: allkeys-lru`.

## Capacity planning cheat-sheet

| Daily queries | API pods | Index storage | Crawler workers | Postgres |
| ------------- | -------- | ------------- | --------------- | -------- |
| 100 k | 3 | 50 GiB | 5 | r6g.xlarge |
| 1 M | 8 | 400 GiB | 25 | r6g.2xlarge |
| 10 M | 25 | 3 TiB | 80 | r6g.4xlarge |
| 100 M | 60 | 25 TiB | 200 | r6g.8xlarge + 3 read replicas |

## Cost optimization

- **Spot instances** for crawler workers (interruptible, re-queue on SIGTERM).
- **S3 Intelligent-Tiering** for WAL-G backups.
- **Index warming:** Pre-load hot segments into page cache via `madvise(MADV_WILLNEED)` on pod start.