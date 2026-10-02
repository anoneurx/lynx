# Rate limits

Rate limiting protects availability. In LYNX it has a second job: bounding what any party can
observe about a single source without retaining the source.

## Token bucket

```text
capacity   = burst allowance
refill     = sustained rate
```

| Client | Capacity | Refill |
| ------ | -------- | ------ |
| Anonymous, `/search` | 10 | 1/s (60/min) |
| Anonymous, `/suggestions` | 30 | 2/s (120/min) |
| Anonymous, `/status` | 10 | 0.2/s (12/min) |
| With key, `/search` | 30 | 1/s (60/min) |
| With key, `/suggestions` | 60 | 10/s (600/min) |
| Operator, `/explain` | 10 | 0.2/s (12/min) |
| `/admin/*` | 60 | 1/s |

Suggestions get a higher limit than search because typeahead fires per keystroke. A client
debouncing at 80 ms and a user typing ten characters generate ten requests, so a limit tuned for
search would throttle ordinary use.

## Identity, without identity

Anonymous requests have no account, so the bucket must key on something. It keys on
`HMAC(ip, daily_salt)`.

| Property | Effect |
| -------- | ------ |
| No raw IP retained | The bucket identifier is a hash |
| Salt rotates daily | Yesterday's identifier cannot be correlated with today's |
| Not reversible | Recovering the IP from a bucket key requires the salt, which lives for a day |
| Not per-user | The bucket counts requests, not people |
| NAT-tolerant | Many users behind one address share a bucket, hence the generous capacity |

The daily salt is the interesting part. With a permanent secret, the bucket identifiers would be
a permanent record of which addresses searched LYNX and when — a search history reconstructed
from the rate limiter, which is precisely the thing the privacy model refuses to create
elsewhere. Rotating it daily means the identifiers are useful for counting and useless as a
record.

```text
salt(t)      = HMAC-SHA256(kms_root, "ratelimit/" || date(t))
bucket_key(t)= HMAC-SHA256(salt(t), ip)
retention    = 48 h, then the counters are deleted regardless
```

## Response headers

```text
X-RateLimit-Limit: 60
X-RateLimit-Remaining: 57
X-RateLimit-Reset: 42
Retry-After: 42                    # on 429 only
```

Clients can display a countdown, and a client that does not read these headers still behaves
correctly because the 429 is well-formed.

## Key-based limits

```bash
POST /api/v1/keys
```

```json
{ "id": "key_01JX…", "prefix": "lynx_ab12",
  "scopes": ["search", "suggestions"],
  "rate_limit": { "requests_per_minute": 60, "burst": 30 },
  "created_at": "2026-02-17T04:00:00Z" }
```

The secret is returned exactly once. Only a hash is stored, so it cannot be retrieved, listed, or
leaked from the database.

A key is an identifier for a developer's traffic, not for a person:

| Recorded | Not recorded |
| -------- | ------------ |
| Key id, prefix | The secret, ever, after creation |
| Request counts, bucketed | Which queries were made |
| Last-used timestamp | Any per-query data |
| Scopes | Any user identity |

A developer can see their own usage volume, and nobody can see what they searched. That boundary
is enforced by the absence of any column to store it.

## Abuse handling

| Abuse | Response |
| ----- | -------- |
| Sustained high volume from one address | Bucket exhaustion; sustained exhaustion raises the limit to 0 |
| Distributed low-rate scraping | Detected by the global egress rate, not per address; a behavioural flag, then a block |
| Crawler traffic | Distinguished by `User-Agent` at the edge, given a separate pool, and never served from the same buckets |
| An API key abused | Revoked immediately; the abuse stops with one call |

Bot traffic gets its own bucket rather than sharing the anonymous one, so a scraper cannot
deplete the budget that real users depend on. That is the same principle as the crawler's
politeness budget: a party that behaves well should not be starved by a party that does not.

## What rate limiting cannot do

| Limit | Reason |
| ----- | ------ |
| Identify a person | No accounts, no identity, no IP retention |
| Distinguish two users behind one NAT address | They share a bucket, deliberately |
| Prevent a distributed slow scrape | Volume limits bound impact, not intent |
| Stop a determined attacker | It raises cost, which is the goal |
| Prove who made a request | There is no evidence to present |

The last row is worth stating plainly. LYNX cannot attribute a request to a person, because it
has no way to know who they are. For an abuse investigation, the answer to "who did this" is
genuinely "we cannot tell you", and that is a consequence of the privacy model rather than a gap
in the implementation.

## Configuration

```toml
[rate_limit]
anonymous_search      = { capacity = 10,  refill_per_second = 1.0 }
anonymous_suggestions = { capacity = 30,  refill_per_second = 2.0 }
anonymous_status      = { capacity = 10,  refill_per_second = 0.2 }
keyed_search          = { capacity = 30,  refill_per_second = 1.0 }
keyed_suggestions     = { capacity = 60,  refill_per_second = 10.0 }
exhausted_backoff     = 3600            # sustained exhaustion → this long a block
salt_rotation_hours   = 24
counter_retention_hours = 48
```

`counter_retention_hours = 48` is deliberately longer than `salt_rotation_hours = 24` so a
bucket survives one rotation, and deliberately short so nothing accumulates into a history.

## Enforcement point

Rate limiting happens at the edge, before the search path. Two reasons:

1. A blocked request must not reach the query handler at all.
2. The limiter must not itself need to parse or log the query.

This means the limiter sees only the derived identifier and the path, never the query
parameters. A rate limiter that logged queries would reintroduce exactly the log the privacy
model forbids, in the component whose entire job is to observe requests.

## Testing

- **Bucket behaviour:** capacity is respected exactly; refill is monotonic; a burst after
  exhaustion is refused.
- **Rotation:** after the salt rotates, the previous identifier produces a fresh bucket and the
  old counters are unreachable.
- **Retention:** counters older than 48 hours are deleted.
- **Privacy:** the limiter's write path can only reach the counter store, and the counter store
  schema has no column for a query — asserted as a type-level property.
- **Isolation:** bot traffic exhausting its pool does not affect the anonymous pool.
- **Headers:** every response carries accurate `X-RateLimit-*` values, including on 429.

## Related

- [errors.md](./errors.md) — the 429 envelope
- [../privacy/privacy-model.md](../privacy/privacy-model.md) — why the salt rotates
- [../privacy/data-flow.md](../privacy/data-flow.md) — counter retention
- [../crawler/politeness.md](../crawler/politeness.md) — the same principle applied to crawling