# `tests/security` — adversarial suites

These are the tests that encode [THREAT_MODEL.md](../docs/threat-model.md). Each suite
names the threat IDs it covers.

| Suite | Threats | Content |
| ----- | ------- | ------- |
| `ssrf` | T02, T17 | Every entry in the blocked-CIDR set, encoded per hop; IPv4-mapped IPv6, 6to4, NAT64, decimal/octal/hex host forms, DNS rebinding simulation, redirect-to-internal chains, IPv6 unique-local, cloud metadata endpoints |
| `flood` | T01 | Token-bucket correctness, salt rotation, bucket expiry, cross-midnight behaviour, concurrency limits, oversized query rejection |
| `injection` | T03, T05 | SQL metacharacters through every query and admin parameter; log-injection newlines; header injection through `Host`/`X-Forwarded-For`; XSS payloads through query echo and snippets |
| `limits` | T11, T12 | Overlong queries, deep boolean nesting, huge phrases, decompression bombs, `Content-Length` lies, mid-stream abort correctness |
| `secrets` | T21 | Repository and image secret scan; no secret in config, logs, metrics, or error messages |
| `headers` | T26 | Exact CSP/HSTS/Referrer-Policy assertions for web and admin; no `unsafe-inline`; no third-party origin |
| `authz` | T07, T08 | API key scope enforcement, key rotation invalidation, admin route rejection without SSO, role matrix |
| `ai` | T06 | Prompt-injection corpus against Astra; unverified-claim suppression; citation fabrication attempt; context-overflow handling (skipped until Phase 5) |
| `privacy` | T25, T26, T28, T29 | Assert that no query substring reaches any log, metric, trace, or DB row; no cookies; no `localStorage` query persistence; access-log path-only format |

The `privacy` suite is a hard CI gate. A failure there blocks merge regardless of severity
labels — see [PRIVACY.md §10](../PRIVACY.md#10-self-audit-checklist).

All suites are hermetic and run against local fixtures only.