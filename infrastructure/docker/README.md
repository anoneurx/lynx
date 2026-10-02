# `infrastructure/docker` — images and local containers

## Production images (multi-stage, BuildKit)

```text
infrastructure/docker/
├── Dockerfile.api         Rust → distroless, non-root, read-only fs
├── Dockerfile.crawler     Rust → distroless, CA bundle only, no shell
├── Dockerfile.indexer     Rust → distroless
├── Dockerfile.ai          Rust → distroless, egress-restricted
├── Dockerfile.web         Node build → static assets served by nginx-unprivileged
├── Dockerfile.admin       Node build → static assets (internal listener)
└── nginx/                 hardened nginx config for static assets only
```

Rules that apply to every image:

- Multi-stage; build toolchain never reaches the runtime layer.
- Base images pinned **by digest** and refreshed by a weekly automated PR.
- `USER` is a non-root uid with no shell (`/sbin/nologin` equivalent).
- `ENTRYPOINT` is the binary directly; no wrapper shell, no `curl`-based healthcheck that
  needs a shell — health is a `/healthz` endpoint.
- No `docker.sock`, no package manager, no dev headers in the final layer.
- OCI labels carry the source revision and build date for provenance.
- SBOM generated with Syft, signed with cosign, verified at admission.

## Local development

`docker-compose.yml` at the repository root wires the same image definitions into a
developer stack: `lynx-web`, `lynx-api`, `lynx-crawler`, `lynx-indexer`, `lynx-db`,
`lynx-cache`, `lynx-monitoring`. See [DEVELOPMENT.md](../../DEVELOPMENT.md).

## Hardening checklist (applied to every image)

- [ ] Pinned base digest
- [ ] Multi-stage, no toolchain in output
- [ ] Non-root user, no shell
- [ ] Read-only rootfs compatible (all writes to explicit volumes)
- [ ] No secrets via `ENV`/`ARG`/build args
- [ ] SBOM attached
- [ ] Trivy scan: 0 HIGH/CRITICAL (HIGH needs a documented waiver)
- [ ] Minimal CA bundle (crawler needs roots, nothing else)
- [ ] `docker scout`/equivalent dependency scan clean