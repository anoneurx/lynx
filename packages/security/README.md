# `packages/security` — security primitives

Reusable, auditable security controls. Every module here is security-critical and carries
two-reviewer requirements on change.

## Modules

| Module | Purpose | Threat coverage |
| ------ | ------- | --------------- |
| `ratelimit` | Rotating salted prefix buckets (v4 `/24`, v6 `/48`), sliding window, per-API-key quotas | T01, T28 |
| `apikey` | Generation (`lnx_` + 32 random bytes, base32), `sha256` storage, constant-time verify, prefix display, rotation | T08 |
| `secrets` | `SecretRef` resolution, `SecretString` wrappers, zeroise on drop, no `Debug` leakage | T21 |
| `headers` | Canonical security header set and the CSP for `apps/web` and `apps/admin` | T26, T29 |
| `blocklist` | Host/domain/IP blocklist matching with wildcards, used for banned domains and the policy term list | T03, T04, T31 |
| `redirect` | Same-origin / cross-origin redirect policy helper (used by the crawler's redirect handler) | T13 |
| `redact` | Log/response redaction helpers shared by all crates | T25, T29 |

## Rate-limit bucket derivation

```text
v4: bucket = hex( HMAC-SHA256( salt_day, ip[0] || ip[1] || ip[2] || 0x00 ) )
v6: bucket = hex( HMAC-SHA256( salt_day, ip[0..6] ) )
salt_day = HMAC-SHA256( server_secret, YYYY-MM-DD )      // rotates every 24 h
TTL      = 25 h, memory-only store
```

Properties this gives us:

- No durable linkable identifier: yesterday's bucket for the same prefix is a different
  value because the salt changed.
- No precision beyond the prefix: we cannot reconstruct an address from a bucket.
- No coordination with clients: no cookie, no header, no account.
- Acceptable cost: a /24 NAT block shares a quota, so a burst from one popular network can
  throttle other users behind it. This is a deliberate availability trade.

## Header policy

`apps/web` (see [docs/security/headers.md](../../docs/security/headers.md)):

```text
Content-Security-Policy:
  default-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors 'none';
  form-action 'self'; connect-src 'self'; img-src 'self' data:;
  font-src 'self'; style-src 'self'; script-src 'self';
  require-trusted-types-for 'script'; upgrade-insecure-requests
Strict-Transport-Security: max-age=63072000; includeSubDomains; preload
Referrer-Policy: no-referrer
X-Content-Type-Options: nosniff
X-Frame-Options: DENY
Permissions-Policy: interest-cohort=(), browsing-topics=(), geolocation=(), camera=(),
                    microphone=(), payment=(), usb=(), accelerometer=(), gyroscope=()
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Resource-Policy: same-origin
Cache-Control (SERP): no-store
```

There is deliberately no `unsafe-inline` for scripts, no third-party origin anywhere, and
no `style-src 'unsafe-inline'` — the build inlines CSS at build time instead.

## Review policy

Changes to this crate require:

1. Two maintainer approvals, one of whom must not be the author.
2. A `security` label and a threat-model cross-reference in the PR description.
3. Unit tests covering the failure case, not just the happy path.
4. A property test for anything that derives a value from user input.