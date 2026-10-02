# Secrets Management

## Threat model

| Asset | Threat | Mitigation |
| ----- | ------ | ---------- |
| Database credentials | Theft from config/git | Never in repo; injected at runtime |
| TLS private keys | Memory scraping | Short-lived certs (cert-manager), HSM for root CA |
| API keys (external) | Leak in logs | Redacted in structured logs, never printed |
| Vault unseal keys | Insider threat | Shamir split (5-of-7), stored offline |

## Architecture

```
┌──────────────────┐     ┌──────────────────┐
│  ExternalSecrets │────▶│  Kubernetes      │
│  Operator        │     │  Secrets         │
└────────┬─────────┘     └────────┬─────────┘
         │                        │
         ▼                        ▼
┌──────────────────┐     ┌──────────────────┐
│  HashiCorp Vault │     │  Workload Pods   │
│  (HA Raft, 3+    │     │  (env / volume)  │
│   nodes)         │     └──────────────────┘
└──────────────────┘
```

## Vault setup

### Auth

- **Kubernetes auth method** (JWT, service account `external-secrets`).
- Policy `lynx-secrets-reader` grants `read` on `secret/data/lynx/*`.

### Secret paths

| Path | Contents | Rotation |
| ---- | -------- | -------- |
| `secret/data/lynx/postgres` | `username`, `password`, `host`, `port` | 90 days |
| `secret/data/lynx/redis` | `password` | 90 days |
| `secret/data/lynx/tls/api` | `tls.crt`, `tls.key`, `ca.crt` | 30 days (cert-manager) |
| `secret/data/lynx/tls/crawler` | `tls.crt`, `tls.key`, `ca.crt` | 30 days |
| `secret/data/lynx/s3` | `access_key`, `secret_key` | 90 days |
| `secret/data/lynx/jaeger` | `agent_host`, `agent_port` | — |

### Example ExternalSecret

```yaml
# deploy/kubernetes/base/external-secret.yaml
apiVersion: external-secrets.io/v1beta1
kind: ExternalSecret
metadata:
  name: lynx-postgres
  namespace: lynx
spec:
  refreshInterval: 1h
  secretStoreRef:
    kind: ClusterSecretStore
    name: vault-backend
  target:
    name: lynx-postgres-credentials
    creationPolicy: Owner
  data:
    - secretKey: username
      remoteRef:
        key: secret/data/lynx/postgres
        property: username
    - secretKey: password
      remoteRef:
        key: secret/data/lynx/postgres
        property: password
```

## Certificates (TLS)

### Internal mTLS

- **cert-manager** + **Vault PKI** (intermediate CA).
- All pod-to-pod: mTLS via Istio / Linkerd or raw TLS.
- Cert lifetime: 24 h, auto-renewed at 2/3.

### Public API (api.lynx.example.com)

- **Let's Encrypt** (HTTP-01) via cert-manager `ClusterIssuer`.
- Cert stored in `secret/data/lynx/tls/api` for nginx-ingress.

## Key rotation

| Secret | Rotation method | Downtime |
| ------ | --------------- | -------- |
| Postgres password | `ALTER ROLE ... PASSWORD`, update Vault, roll pods | Zero (pool re-auth) |
| Redis password | `CONFIG SET requirepass`, update Vault, rolling restart | < 30 s |
| TLS certs | cert-manager → Vault → ExternalSecrets → pod reload | Zero (in-place) |
| S3 keys | IAM key rotation, update Vault | Zero |
| Vault unseal | `vault operator rekey` (offline) | Requires quorum |

Automation: `lynx-rotate-secrets` cronjob (daily check, rotates if > 80 % TTL).

## Local development

```bash
# .env.local (gitignored)
LYNX_DATABASE_URL=postgres://lynx:lynx@localhost:5432/lynx
LYNX_REDIS_URL=redis://localhost:6379
LYNX_INDEX_PATH=/tmp/lynx-index
```

`direnv` + `.envrc` loads automatically. No Vault needed locally.

## CI/CD

- **GitHub Actions** uses OIDC to assume `ci-deploy` IAM role.
- Short-lived tokens (15 min) for: container registry push, Kustomize apply.
- No long-lived credentials in repo or runner.

## Audit

- **Vault audit device** → CloudWatch Logs (immutable, 1 year).
- Alert on: `path="secret/data/lynx/*" operation="delete"`, `response.code=403`.

## Emergency access

Break-glass procedure (documented in [RB-07](../security/RB-07.md)):

1. On-call + security lead approve via PagerDuty.
2. Retrieve unseal shares from offline safe (5-of-7).
3. `vault operator unseal` each share.
4. Generate temporary root token (`vault token create -policy=root -ttl=1h`).
5. Perform fix, revoke token, re-seal.

All steps logged, reviewed in post-incident.