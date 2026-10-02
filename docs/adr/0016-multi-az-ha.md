# ADR 0016: Multi-AZ High Availability

**Date:** 2024-06-01
**Status:** Accepted
**Deciders:** SRE, Platform Team
**Tags:** ha, multi-az, reliability

## Context

SLA: 99.9% availability (8.77 hrs downtime/year).
Single-AZ failure (power, network, cooling) must not cause outage.

## Decision

**Active-active across 3 AZs** (us-east-1a, 1b, 1c).

### Topology

| Component | AZ Distribution | Failover |
| --------- | --------------- | -------- |
| `lynx-api` | 3+ pods per AZ (HPA) | NLB cross-zone |
| `lynx-crawler` | Spot instances across AZs | Queue persists (Redis) |
| `lynx-indexer` | 1 leader (any AZ) | Patroni lock → new leader |
| `lynx-postgres` | Primary in 1a, sync replicas in 1b, 1c | Patroni auto-failover (< 30s) |
| `lynx-redis` | 2 shards per AZ (6 total) | Cluster mode, auto-rebalance |
| Tantivy (EFS) | Regional (multi-AZ) | N/A (shared) |
| S3 | Cross-region replication | N/A |

### Network

- **NLB** (cross-zone load balancing enabled) → distributes to all healthy pods
- **Pod anti-affinity** → `topologyKey: topology.kubernetes.io/zone`
- **PDB** → `minAvailable: 80%` per AZ

### Postgres Failover

```yaml
# Patroni config
postgresql:
  parameters:
    synchronous_commit: "remote_apply"
    synchronous_standby_names: "FIRST 1 (replica_1b, replica_1c)"
```

- **RPO = 0** (synchronous replication to 1 replica)
- **RTO < 30s** (Patroni promotes replica, updates DNS/Endpoints)

### Crawler Resilience

- Queue in Redis Streams (persisted, replicated)
- Workers stateless → reschedule on any AZ
- Politeness in Redis (survives worker loss)

### Indexer Resilience

- Leader lock in Postgres (survives AZ loss)
- Build progress in Postgres → new leader resumes
- Segments on EFS (multi-AZ)

## Consequences

### Positive
- **AZ failure = no user impact** (cross-zone NLB, synchronous PG)
- **Regional failure** → DR plan (cross-region S3, Vault)

### Negative
- **Cost**: ~2.5× single-AZ (cross-zone traffic, replica storage)
- **Latency**: Cross-AZ ~1-2ms (negligible for search)
- **Complexity**: Patroni, Redis Cluster, NLB config

## DR (Region Failure)

| Asset | RPO | RTO | Mechanism |
| ----- | --- | --- | --------- |
| Postgres | < 5s | 15 min | Cross-region read replica promotion |
| Tantivy | 0 | 5 min | S3 cross-region replication + EFS mount |
| Vault | 0 | 10 min | Cross-region raft replica |
| Secrets | 0 | 5 min | ExternalSecrets re-sync |

## Alternatives Rejected

| Approach | Reason |
| -------- | ------ |
| Single-AZ + fast recovery | RTO too high for 99.9% |
| Active-passive (warm standby) | Wasteful; failover not tested continuously |
| Multi-region active-active | Cost 5×; consistency complexity |

## Related

- ADR 0005: Postgres (Patroni)
- ADR 0006: Redis (Cluster)
- ADR 0009: Kubernetes (topology spread)