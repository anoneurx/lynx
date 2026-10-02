# ADR 0009: Kubernetes for Production Deployment

**Date:** 2024-03-01
**Status:** Accepted
**Deciders:** Platform Team, SRE
**Tags:** kubernetes, deployment, orchestration

## Context

Production requirements:
- Zero-downtime deploys
- Auto-scaling (HPA, VPA)
- Self-healing (restart on crash, reschedule on node failure)
- Multi-AZ HA
- GitOps workflow
- Secret management integration

Options: Kubernetes (EKS/GKE/AKS/self-hosted), Nomad, ECS, VMs + systemd, Fly.io/Render.

## Decision

**Kubernetes (managed: EKS/GKE)** for all production workloads.

## Consequences

### Positive
- **Industry standard** → talent, tooling, ecosystem
- **Declarative GitOps** (ArgoCD/Flux) → audit trail, rollback
- **HPA/VPA** → automatic scaling per workload
- **PodDisruptionBudgets** → controlled rollouts
- **Network policies** → zero-trust between planes
- **CSI drivers** → EFS for Tantivy, EBS for Postgres/Redis
- **IRSA/OIDC** → fine-grained IAM for pods (S3, Vault)

### Negative
- **Complexity** → steep learning curve, operational burden
- **Cost** → control plane (~$73/mo EKS), overprovisioning risk
- **Upgrade cadence** → version skew management

## Architecture

```
Git (main) → ArgoCD → Kustomize overlays → EKS
                    │
                    ├── base/ (common)
                    ├── overlays/production/
                    ├── overlays/staging/
                    └── overlays/canary/
```

### Key Resources

| Resource | Replicas | Scaling | PDB |
| -------- | -------- | ------- | --- |
| `lynx-api` | 3-50 | HPA (QPS, latency) | minAvailable 80% |
| `lynx-crawler` | 5-200 | HPA (queue depth) | minAvailable 50% |
| `lynx-indexer` | 1 (leader) | Manual | minAvailable 0 |
| `lynx-postgres` | 3 (Patroni) | Manual | N/A (StatefulSet) |
| `lynx-redis` | 6 shards | Manual | N/A |

## Alternatives Rejected

| Platform | Reason |
| -------- | ------ |
| Nomad | Smaller ecosystem, no managed offering |
| ECS | AWS lock-in, less flexible scaling |
| VMs + systemd | No auto-scaling, no self-healing, manual HA |
| Fly.io/Render | Cost at scale, less control, vendor lock-in |

## Related

- ADR 0004: Index Reload (K8s rollout)
- ADR 0015: GitOps (ArgoCD)
- ADR 0016: Multi-AZ HA