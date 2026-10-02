# Security Policy — LYNX

Reporting instructions first; policy details after. The full design-level model is in
[docs/threat-model.md](./docs/threat-model.md) and [docs/security/](./docs/security/README.md).

## Reporting a vulnerability

**Do not open a public issue for a security problem.**

- Preferred: GitHub Security Advisories ("Report a vulnerability") on this repository.
- Alternative: security@anoneurx.com
- Also possible: [anoneurx.com/security](https://anoneurx.com/security)

Please include: affected component or version, what an attacker gains, reproduction steps or
a proof of concept, and any suggested mitigation.

### What to expect

| Stage | Commitment |
| ----- | ---------- |
| Acknowledgement | 48 hours |
| Initial assessment (severity, acknowledgement of validity) | 5 business days |
| Fix or mitigation for critical/high | 14 days |
| Fix or mitigation for medium | 30 days |
| Fix or mitigation for low | next scheduled release |
| Public disclosure | 90 days after the fix ships, coordinated with you |

You will be credited in the release notes unless you prefer otherwise. We do not pursue
legal action against good-faith researchers, and we do not require a non-disclosure
agreement to accept a report.

### In scope

Crawler SSRF and sandbox escapes, injection of any kind, authentication or authorization
flaws, API-key compromise, secret exposure, privacy-model violations (query or personal
data persisting where the model says it does not), index tampering, denial of service
against the query plane, supply-chain compromise of shipped artefacts, and admin-plane
exposure.

### Out of scope

Social engineering of maintainers, physical attacks, missing hardening headers with no
demonstrable impact, DoS requiring the reporter to already hold credentials, self-inflicted
misconfiguration of a test environment, and reports generated solely by an automated
scanner without evidence. We will tell you when something falls in the last category
rather than silently closing it.

## Supported versions

| Version | Supported |
| ------- | --------- |
| latest release | yes |
| previous minor | security fixes only |
| anything older | no |

## Security design commitments

These are architectural commitments, not marketing. Each maps to a control described in
the linked document.

1. **Crawled content is untrusted input.** Every byte from the web passes through a
   validation, size, and time limit before it can consume memory or reach the network.
   See [docs/crawler/safety.md](./docs/crawler/safety.md).
2. **The crawler cannot reach internal infrastructure.** Scheme, port, resolved-IP
   validation against a deny-list of private, loopback, link-local, metadata and
   IPv6-embedding ranges, re-validated at every redirect hop, with DNS pinned to the
   validated address. Defence in depth: the crawl plane also has a network policy denying
   all egress except 80/443 and DNS. See
   [docs/security/ssrf-defense.md](./docs/security/ssrf-defense.md).
3. **No user profiling.** No accounts, no cookies, no client identifiers, no query logging.
   See [PRIVACY.md](./PRIVACY.md).
4. **No third-party requests from the web app.** Enforced by CSP and asserted by tests.
5. **Explainable ranking.** Integrity of results is a security property too: no signal
   enters the ranker without a name, a range, a weight, and a test.
6. **Reproducible, scanned, signed artefacts.** Pinned digests, SBOM, Trivy, `cargo-deny`,
   `cargo-audit`, gitleaks, cosign signatures, and CI actions pinned by SHA.
7. **Least privilege by database role.** The crawler cannot write audit records; the API
   cannot enqueue URLs; the AI service cannot reach the crawl plane.
8. **Admin is not public.** VPN + SSO + MFA, audited, with dual-control break-glass.
9. **Documented, rehearsed incident response.** Runbooks in
   [docs/security/incident-response.md](./docs/security/incident-response.md); a post-incident
   ADR is mandatory after any security incident.
10. **Honest disclosure of limits.** [docs/threat-model.md](./docs/threat-model.md) states
    residual risks explicitly rather than claiming elimination.

## Default security behaviours you can rely on

| Behaviour | Where |
| --------- | ----- |
| TLS 1.3 minimum, HSTS preloaded | [docs/security/headers.md](./docs/security/headers.md) |
| Rate limiting without durable identifiers | [docs/privacy/model.md](./docs/privacy/model.md) |
| Query strings never written to logs, metrics, or storage | [docs/privacy/telemetry.md](./docs/privacy/telemetry.md) |
| AI answers fail closed without verified citations | [docs/ai/grounding.md](./docs/ai/grounding.md) |
| Prompt-injection defences by architecture, not prompt wording | [docs/ai/prompt-injection.md](./docs/ai/prompt-injection.md) |
| Migrations are expand/contract and never lock long | [docs/database/migrations.md](./docs/database/migrations.md) |

## Hardening for operators

Running LYNX yourself: keep the defaults. Specifically, do not disable `respect_robots`,
do not lower TLS minimums, do not set `trusted_proxy_depth` unless you control every proxy
in front of you (a wrong value lets clients forge their own rate-limit bucket), and do not
change anything under `[privacy]` without reading [PRIVACY.md §8](./PRIVACY.md#8-configuration-that-can-weaken-this-model).

## Bugs and non-security issues

Use the bug template. It explicitly asks for **query shape** rather than query text — we do
not need your search to fix a ranking bug, and we would rather not have it.