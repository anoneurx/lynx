# Privacy Model — LYNX

> Privacy is a design constraint of the architecture, not a policy applied on top of it.
> Where this document says "we do not store X", the architecture is arranged so that
> storing X is not possible on the query path without an explicit code change.

Read also: [docs/privacy/model.md](./docs/privacy/model.md),
[docs/privacy/data-inventory.md](./docs/privacy/data-inventory.md),
[docs/privacy/limitations.md](./docs/privacy/limitations.md).

## 1. Position

LYNX is a privacy-focused independent search engine. It aims to answer queries without
building a durable record of who asked what, and without sharing user data with anyone.

LYNX does **not** claim to make users anonymous. Read
[docs/privacy/limitations.md](./docs/privacy/limitations.md) before making any privacy
claim about LYNX. Anonymity is a network-layer property; privacy of search is a
data-minimisation property. LYNX implements the second and is explicit about the first.

## 2. The five rules

1. **No query text is persisted by LYNX.** Not in Postgres, not in Redis, not in logs, not
   in metrics labels, not in analytics.
2. **No cookies, no local identifiers, no fingerprinting** on any LYNX surface.
3. **No third-party requests.** No external fonts, scripts, analytics, pixels, CDNs, or
   error reporters. First-party origin only, enforced by CSP.
4. **No advertising, no data brokerage, no sale of anything.**
5. **Every stored field has a documented purpose, retention period, and owner.** Anything
   that fails this test is not collected. See
   [docs/privacy/data-inventory.md](./docs/privacy/data-inventory.md).

## 3. What happens to a query

```text
request arrives
  → TLS terminated at edge
  → edge access log written with PATH ONLY (never query string)      ← configured rule
  → privacy middleware derives rate-limit bucket
       bucket = HMAC-SHA256(daily_salt, /24_prefix_of_client_ip)
       daily_salt = HMAC-SHA256(server_secret, UTC date)
       TTL 25h, memory-only Redis, never flushed to disk
  → query string lives in process memory for the duration of the request
  → cache key = HMAC-SHA256(cache_salt, normalised_query)   ← plaintext never stored
  → response returned, memory freed
  → metrics: length bucket, token count, operator classes, latency, status
       (no query text, no IP, no session id, no user agent fingerprint)
```

Consequences worth stating plainly:

- Two identical queries from two people in the same /24 network share a rate-limit bucket.
  We cannot rate-limit an individual without identifying them, and we chose not to identify.
- The API response cache stores *public web results* keyed by a *hash*. Nobody, including
  us, can recover the query from cache contents.
- We do not need a query log to operate. Every operational signal we rely on is
  available in aggregate shape form.

## 4. Rate limiting without identity

Two implementation options exist. The privacy-preserving one is chosen:

| Approach | What it needs | Acceptable? |
| -------- | ------------- | ----------- |
| Per-IP token bucket in durable storage | Stable client identifier | No |
| Per-account quota | Accounts | No — accounts are profiles |
| **Rotating salted prefix bucket (chosen)** | Nothing but the connection's source address | Yes |

Mechanics, defined in [ADR-0005](./docs/adr/0005-privacy-model.md):

- Bucket key is `HMAC-SHA256(salt_day, first_3_octets_of_IP)`.
- `salt_day` derives from a secret held in the secret store, rotated by UTC date.
- Buckets expire after 25 hours; Redis runs with `appendonly no`, `save ""`, no swap
  expectation for this dataset.
- IPv6 uses the /48 prefix rather than /24 to limit precision.
- NAT and VPN users share buckets; this is an accepted availability cost, not a bug.

Bypass resistance is limited to address-space guessing, which does not help an attacker
who already needs to avoid detection. See
[docs/threat-model.md](./docs/threat-model.md#2-external-attacker-threats).

## 5. Data LYNX deliberately does not collect

| Data | Why not |
| ---- | ------- |
| Search history | Re-identification of sensitive interests; no operational need |
| Accounts / logins | Identity is not required to serve a query |
| Cookies, advertising IDs | Tracking infrastructure has no place here |
| Persistent device IDs, canvas/font fingerprinting | Deceptive fingerprinting |
| Exact client IP in application storage | Unnecessary precision; /24 prefix with daily salt is enough for abuse control |
| User-Agent strings retained | Not needed; used transiently for content negotiation only |
| Referrer chains from users | Not collected |
| Cross-site identifiers | We operate no other site |
| Behavioural analytics, heatmaps, session replay | Monitoring is aggregate-only |

## 6. Data LYNX does store, with retention

| Category | Example | Retention | Notes |
| -------- | ------- | --------- | ----- |
| Rate-limit counters | Salted /24 hash | 25 h | Memory only |
| Abuse reports | Reporter contact (only if submitted) | 90 days after resolution | Encrypted at rest; required to act on abuse |
| API keys (developer tier) | `sha256(key)`, prefix, owner email | Life of key + 30 days | Needed for quota and revocation |
| Crawl state | URLs, statuses, robots | Life of URL in index | Contains third-party content metadata, not user data |
| Extracted page text | Searchable text per URL | Refreshed on re-crawl; deleted on deindex/takedown | Third-party copyright and personal data handled in [docs/privacy/legal.md](./docs/privacy/legal.md) |
| Metrics | Aggregated counters/histograms | 30 days raw, 13 months downsampled | No query text, no IP labels |
| Logs | Structured JSON, `request_id` only | 14 days | Query strings redacted at the writer |
| Audit log (admin) | Actor, action, target | 2 years | Required for admin accountability |
| Backups | Postgres + index snapshots | 35 days encrypted | Cannot be selectively purged — documented |

The authoritative inventory with field-level detail is
[docs/privacy/data-inventory.md](./docs/privacy/data-inventory.md).

## 7. Controls

- **Encryption in transit** — TLS 1.3 required, TLS 1.2 only where compatibility demands,
  HSTS preloaded, no plaintext service-to-service traffic in production.
- **Encryption at rest** — volume-level encryption for all persistent stores; field-level
  AES-256-GCM for abuse-report contact data and API key secrets via the application layer.
- **Access control** — four roles (`public`, `developer`, `operator`, `admin`), least
  privilege, admin console behind VPN + SSO, break-glass credentials in the secret store
  with dual control, every privileged read logged to the audit trail.
- **Deletion** — URL-level purge job removes document, index entry, and link edges;
  `takedown_request` table drives automated removal; GDPR/erasure requests handled from
  abuse-report records since no user account table exists.
- **Minimisation** — schema review gate in PR template; every new column needs a row in
  the data inventory or it does not merge.
- **Vendor review** — no third-party processors on the query path. Any future processor
  requires a documented DP decision and a signed agreement before code review is unblocked.

## 8. Configuration that can weaken this model

Every item below is a privacy-relevant switch, namespaced `privacy.*`, with safe defaults.
Weakening any of them requires an ADR.

| Setting | Default | Effect if changed |
| ------- | ------- | ----------------- |
| `privacy.query_logging` | `false` | Writes plaintext queries to the log sink. Forbidden. |
| `privacy.metrics_query_shapes` | `true` (shapes only) | Shape-only telemetry; text still never leaves memory. |
| `privacy.rate_limit.prefix_bits` | `24` (v4) / `48` (v6) | Fewer bits = coarser buckets = more shared buckets. |
| `privacy.rate_limit.salt_ttl` | `24 h` | Longer TTL = buckets linkable across days. |
| `privacy.response_cache.enabled` | `true` | Disabling removes the hashed-key cache entirely. |
| `privacy.third_party_assets` | `forbidden` | CSP-enforced; enabling requires breaking the no-third-party rule. |
| `privacy.tls_min_version` | `1.3` | Weakening changes the traffic-fingerprint surface. |

## 9. User-facing commitments

Published at `/privacy` on the LYNX site, and versioned alongside the code:

1. We do not log your search queries.
2. We do not use cookies or any cross-site identifier on LYNX.
3. We do not load third-party resources.
4. We do not sell, rent, or share your data. There is no mechanism to do so.
5. We do not require an account to search.
6. We publish the exact set of fields we retain, with retention periods.
7. We honour URL takedown requests and provide a contact route.
8. We will publish changes to this model before they ship, with a dated changelog entry.

## 10. Self-audit checklist

Run before every release that touches the query path:

- [ ] No new column in a query-path table that is missing from the data inventory.
- [ ] No log statement receives the raw query; the `redact_query` wrapper is used everywhere.
- [ ] No metric label derives from query text, URL, document id, or IP.
- [ ] CSP still has no third-party origins in any directive.
- [ ] No new outbound request from the web app to a non-first-party host.
- [ ] Redis persistence remains disabled for the rate-limit namespace.
- [ ] Edge/proxy access log format still redacts query strings (integration test).
- [ ] Backup retention and purge behaviour unchanged.
- [ ] Any new third-party dependency audited for telemetry behaviour.

## 11. Related

- [THREAT_MODEL.md](./docs/threat-model.md) — privacy threats T25–T31 (see §5)
- [docs/security/architecture.md](./docs/security/architecture.md) — technical controls
- [docs/privacy/retention.md](./docs/privacy/retention.md) — purge jobs and schedules
- [docs/operations/observability.md](./docs/operations/observability.md) — what we do measure