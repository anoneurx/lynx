# `infrastructure/kubernetes` — Helm charts

```text
infrastructure/kubernetes/
├── charts/
│   ├── lynx/              umbrella chart for the whole platform
│   └── lynx-monitor/      monitoring stack
└── overlays/
    ├── staging/
    └── production/
```

## Design rules

### Workload separation mirrors the plane separation

| Deployment | Plane | Network policy |
| ---------- | ----- | -------------- |
| `lynx-web`, `lynx-api` | query | ingress from ingress controller; egress to cluster only |
| `lynx-crawler`, `lynx-frontier` | crawl | **no ingress**; egress restricted to 80/443 + DNS |
| `lynx-indexer` | crawl | no ingress; egress to DB + index volume only |
| `lynx-ai` | query (sidecar-free) | egress to exactly one model endpoint |
| `lynx-admin` | management | ingress only from the VPN gateway; `noindex` headers |

The crawler's egress restriction is a defence-in-depth layer on top of the in-process
safety gate. Both must exist: the application gate protects against a logic bug, the
network policy protects against a process-level compromise.

### Standard pod settings

```yaml
securityContext:
  runAsNonRoot: true
  runAsUser: 10001
  readOnlyRootFilesystem: true
  allowPrivilegeEscalation: false
  capabilities: { drop: ["ALL"] }
  seccompProfile: { type: RuntimeDefault }
```

plus an `emptyDir` for any scratch space, and a projected service-account token with
`automountServiceAccountToken: false` for workloads that do not need the API.

### Resilience

- **HPA** on CPU + a custom queue-depth metric for the crawler.
- **PDB** with `minAvailable` so a node drain cannot empty a plane.
- **Anti-affinity / topology spread** across zones for the API and index replicas.
- **Graceful shutdown** with a drain period longer than the in-flight request deadline;
  SIGTERM stops frontier claims first, then finishes fetches.
- **Startup probe** distinct from liveness, so a slow index load is not killed.
- **Readiness gates** on index availability: an API replica with an unloaded index does not
  receive traffic.

### Index lifecycle

Index shards are deployed as **separate deployments with their own PVCs** (an index is a
file, not a shared database). Rolling a new index generation is:

```text
build new shard (indexer) → snapshot → deploy as versioned shard
  → API replicas load it → health check → atomic switch of the active shard set
  → keep the previous generation for 24 h → delete
```

This gives index rollback for free and is documented in
[docs/operations/runbooks.md](../../docs/operations/runbooks.md).