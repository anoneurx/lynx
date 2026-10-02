# CI — LYNX

Design: fast feedback on every push, deeper gates on the default branch and before a
release. Secrets are never available to workflows triggered by forked pull requests.

| Workflow | Trigger | Runs | Gate |
| -------- | ------- | ---- | ---- |
| [`ci.yml`](./ci.yml) | push, PR | fmt, clippy, tsc, lint, unit + integration + security tests, config/link/privacy checks | required |
| [`quality.yml`](./quality.yml) | PR → `main` | ranking golden diff + relevance/quality metric gates | required |
| [`e2e.yml`](./e2e.yml) | push → `main`, nightly | Playwright against the full local stack | required on `main` |
| [`fuzz.yml`](./fuzz.yml) | nightly, manual | bounded `cargo-fuzz` over parser, URL, query, safety gate | advisory, fails the build |
| [`perf.yml`](./perf.yml) | nightly, release tag | k6 search load, indexer/crawler throughput, index size | release gate |
| [`supply-chain.yml`](./supply-chain.yml) | push, dependency change | `cargo-deny`, `cargo-audit`, Trivy fs + image scan | required |
| [`images.yml`](./images.yml) | push to `main`, tags | multi-arch build, SBOM (Syft), Trivy, cosign sign & attest | required |
| [`deploy-staging.yml`](./deploy-staging.yml) | merge to `main` | Helm deploy to staging + smoke tests | required |
| [`deploy-production.yml`](./deploy-production.yml) | signed tag, manual approval | Helm deploy, index generation switch, smoke + rollback gate | manual |
| [`docs.yml`](./docs.yml) | PR | markdownlint, link check, mermaid render | required |

## Non-negotiable CI rules

1. **No network egress to the public internet from tests.** Crawler tests use the local
   fixture origin server only. This is enforced by the test harness, not by convention.
2. **Fork PRs run with no secrets and no write permissions.** A workflow touching an
   untrusted `pull_request` gets a read-only token and an empty secret context.
3. **Actions are pinned by commit SHA**, never by tag. Tags are mutable.
4. **No self-hosted runners for untrusted PRs.**
5. **Concurrency groups** prevent overlapping deploys per environment.
6. **Failure paths matter.** Every deploy workflow includes a rollback step that runs on
   failure, not just a success path.
7. **Secrets are never echoed**, never passed as command-line arguments, never written to
   build artifacts. Use OIDC where a cloud provider supports it.
8. **The privacy guard runs on every PR.** `scripts/check-privacy.sh` plus the
   `tests/security/privacy` suite. A failure blocks merge regardless of labels.

## Labels used by CI

```text
ci            required CI check
deps           dependency update
deps-runtime  changes the dependency tree of a shipped binary
release-ok     release lane approved
no-ci          documented exception (needs a maintainer to set, recorded in the PR)
```