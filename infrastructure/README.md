# `infrastructure/` — deployment definitions

Infrastructure is code, versioned here, applied through reviewed pipelines. No production
change is made by hand.

| Path | Tool | Scope |
| ---- | ---- | ----- |
| [`docker/`](./docker/) | Docker BuildKit | Base images, multi-stage builds, local dev containers |
| [`kubernetes/`](./kubernetes/) | Helm | Production workloads, policies, autoscaling |
| [`terraform/`](./terraform/) | OpenTofu / Terraform | Cloud resources: networks, storage, databases, DNS |
| [`monitoring/`](./monitoring/) | Prometheus · Grafana · Alertmanager | Scrapes, recording rules, alerts, dashboards |

## Environment matrix

| Environment | Provisioned by | Shape |
| ----------- | -------------- | ----- |
| `local` | `docker compose` (repo root) | Full stack on one machine |
| `dev` | Docker Compose / k3d | Shared, non-public |
| `staging` | Helm on a small cluster | Production-shaped, synthetic data only |
| `production` | Helm + Terraform | See [DEPLOYMENT.md](../../DEPLOYMENT.md) |

## Principles

1. **Network policies are part of the app.** The crawl plane gets a deny-by-default egress
   policy (80/443 + DNS only) and no ingress from outside the cluster. This is what stops
   a compromised crawler from pivoting into the query plane.
2. **Images are pinned by digest**, built in CI, signed with cosign, scanned with Trivy,
   and accompanied by an SBOM. `latest` is not deployable.
3. **No secrets in the repository or in the image.** Secret values enter through the
   secret manager at runtime.
4. **Every workload runs as non-root** with a read-only root filesystem, dropped
   capabilities, and resource requests/limits.
5. **Infrastructure changes are reviewed like code.** Terraform plan output is attached to
   the PR.

## Related

- [DEPLOYMENT.md](../../DEPLOYMENT.md) — stages and what changes at each
- [docs/security/supply-chain.md](../../docs/security/supply-chain.md)
- [docs/operations/disaster-recovery.md](../../docs/operations/disaster-recovery.md)