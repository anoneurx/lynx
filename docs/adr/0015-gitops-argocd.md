# ADR 0015: GitOps with ArgoCD

**Date:** 2024-05-15
**Status:** Accepted
**Deciders:** Platform Team, SRE
**Tags:** gitops, argocd, deployment

## Context

Deployment requirements:
- Audit trail (who, what, when)
- Rollback in < 1 min
- Environment parity (staging = production config)
- PR-based preview environments
- Drift detection + auto-sync

Options: ArgoCD, Flux, Spinnaker, Jenkins + kubectl, GitHub Actions + kubectl.

## Decision

**ArgoCD** with **Kustomize** for environment overlays.

### Repository Structure

```
deploy/
├── kubernetes/
│   ├── base/                    # Common resources
│   │   ├── deployment-api.yaml
│   │   ├── deployment-crawler.yaml
│   │   ├── statefulset-postgres.yaml
│   │   ├── kustomization.yaml
│   │   └── ...
│   ├── overlays/
│   │   ├── production/
│   │   │   ├── kustomization.yaml  # replicas, resources, images
│   │   │   └── secrets.yaml        # ExternalSecret refs
│   │   ├── staging/
│   │   └── canary/
│   └── apps/                      # ArgoCD Applications
│       ├── lynx-api.yaml
│       ├── lynx-crawler.yaml
│       └── lynx-infra.yaml
```

### ArgoCD Application (example)

```yaml
# deploy/kubernetes/apps/lynx-api.yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: lynx-api
  namespace: argocd
spec:
  project: lynx
  source:
    repoURL: https://github.com/lynx/lynx.git
    targetRevision: main
    path: deploy/kubernetes/overlays/production
    kustomize:
      images:
        - ghcr.io/lynx/api:latest
  destination:
    server: https://kubernetes.default.svc
    namespace: lynx
  syncPolicy:
    automated:
      prune: true
      selfHeal: true
      allowEmpty: false
    syncOptions:
      - CreateNamespace=true
      - PrunePropagationPolicy=foreground
    retry:
      limit: 5
      backoff:
        duration: 5s
        factor: 2
        maxDuration: 3m
```

### Workflow

1. **PR opened** → CI builds images → tags `ghcr.io/lynx/api:pr-123`
2. **ArgoCD** creates preview app `lynx-api-pr-123` in `lynx-pr-123` namespace
3. **PR merged** → CI promotes image to `ghcr.io/lynx/api:sha-abc123`
4. **ArgoCD** detects new image in `production` overlay → auto-sync (if `automated: true`)
5. **Rollback**: `argocd app rollback lynx-api <revision>` or revert Git commit

### Secrets

- **No secrets in Git** → `ExternalSecret` resources reference Vault
- ArgoCD syncs `ExternalSecret` → Operator creates K8s Secret → Pod consumes

## Consequences

### Positive
- **Single source of truth** (Git)
- **Declarative** → desired state = actual state
- **Self-heal** → drift corrected automatically
- **RBAC** → ArgoCD projects map to teams

### Negative
- **ArgoCD operational burden** (control plane, upgrades)
- **Learning curve** for developers (Kustomize, ArgoCD UI)

## Alternatives Rejected

| Tool | Reason |
| ---- | ------ |
| Flux | Similar; ArgoCD has better UI, multi-cluster |
| Spinnaker | Overkill; pipeline-focused not GitOps |
| GitHub Actions + kubectl | No drift detection, no audit trail, manual rollback |

## Related

- ADR 0009: Kubernetes Deployment
- ADR 0006: Secrets Management (ExternalSecrets)