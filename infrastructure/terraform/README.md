# `infrastructure/terraform` — cloud resources

OpenTofu/Terraform modules for everything that is not Kubernetes manifests.

| Module | Provisions |
| ------ | ---------- |
| `network` | VPC/subnets across ≥2 zones, private subnets for data planes, NAT egress only where required, security groups |
| `dns` | public and internal zones, DNSSEC, short TTL for failover, no wildcard analytics |
| `postgres` | managed Postgres, multi-AZ, PITR, TLS enforced, parameter tuning per `docs/operations/capacity.md` |
| `redis` | cache + rate-limit store, persistence disabled, TLS, eviction policy documented |
| `object-store` | bucket for index snapshots and database backups, SSE-KMS, versioning, lifecycle rules for the 35-day retention cap |
| `kms` | key material, rotation, separate keys per data class, break-glass access policy |
| `ingress` | load balancer + TLS certificates (ACME, automated renewal), no query-string logging |
| `network-policy` | Kubernetes network policies as code: crawl-plane egress deny-by-default |
| `observability` | metrics/log/trace backends, retention config, alerting routing |
| `secrets` | secret manager entries and rotation schedule; no secret values in state (encrypted backend) |

## Rules

1. **State is encrypted and remote-locked.** Backend is an encrypted object-store backend
   with locking; no local state files, ever.
2. **No secret values in state or variables.** Modules take *references*; values are
   supplied at apply time from the secret manager.
3. **Plan is reviewed.** `tofu plan` output is attached to every PR. Applying to
   production requires the environment-protected branch and a second approval.
4. **Modules are versioned and pinned.** Shared modules come from the internal registry at
   an immutable tag.
5. **No hardcoded region/account identifiers** in modules — all via variables.
6. **Destroying the database is protected** by `prevent_destroy` on the production module
   instance and a Terraform variable confirmation token.
7. **Everything is tagged** (`project=lynx`, `env`, `owner=platform`, `cost-center`) so
   the bill is attributable and the blast radius of a credential leak is knowable.

## Access logging — explicit

The ingress and any proxy are configured with a **path-only** access log format. Query
strings are never written. This is not a default; it is an assertion in this module and a
test in CI, because it is the single most likely place for a privacy regression to be
introduced silently. See [PRIVACY.md §3](../../PRIVACY.md#3-what-happens-to-a-query).