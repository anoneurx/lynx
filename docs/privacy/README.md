# Privacy — commitments and their enforcement

| Document | Contents |
| -------- | -------- |
| [privacy-model.md](./privacy-model.md) | The commitments, what each costs, what is never collected |
| [data-flow.md](./data-flow.md) | Every store, its retention, access, and deletion procedure |
| [data-minimisation.md](./data-minimisation.md) | The reasoning layer, deliberate omissions, future-data tests |
| [crawler-ethics.md](./crawler-ethics.md) | Robots compliance, attribution, opt-out, rate limiting, DMCA |
| [self-hosting.md](./self-hosting.md) | Operating an instance without becoming a surveillance system |

The full adversarial analysis is [../threat-model.md](../threat-model.md). This directory is
about what LYNX deliberately does not do.

## The short version

```text
no query logging        enforced by there being no write path
no cookies              no user model exists
no third parties        CSP with no external origins, egress allowlist
no user accounts        no identity to protect
no personalisation      no profile store
no raw HTML retention   discarded after parsing
no IP retention         bucketed for rate limiting with a rotating salt
```

## Why it is structured this way

A privacy policy is a statement; a privacy model is a set of constraints that make the
statement hard to violate accidentally. Each commitment above has a corresponding
structural mechanism, and each mechanism is testable:

| Commitment | Structural mechanism | Test |
| ---------- | -------------------- | ---- |
| No query logging | No persistence call in the search path | Static check plus a runtime assertion on Redis writes |
| No third parties | Same-origin CSP, no external asset | Build-time asset-URL check |
| No IP retention | Rotating-salt bucketing | Key-rotation test |
| No raw HTML | Parse-then-discard | No store for HTML exists in the schema |
| No personalisation | No profile table | No such table in the schema |

The pattern is that a violation requires someone to add a code path, not merely to forget a
line. That is the standard worth holding: a design where compliance requires vigilance will
eventually fail through inattention.

## What it costs

| Cost | Real? | Assessment |
| ---- | ----- | ---------- |
| Weaker spelling correction | Yes | Dictionary and corpus statistics instead of real misspellings |
| No learning-to-rank | Yes | The largest quality lever in search, unavailable |
| No trending topics | Yes | Inherently a query-log product |
| No personalised results | Yes | The core feature of every commercial engine |
| No cached pages | Yes | Users notice this more than any other |
| Weaker coverage | Partly | No bulk seed corpus or purchased feeds |
| No per-query debugging | Yes | Compensated by `/explain` and the plan serialisation |

Six real costs, each accepted knowingly. The alternative — collecting the data and writing a
policy promising restraint — is the industry norm, and it is the thing being rejected.

## What an operator cannot see

Privacy that only constrains third parties is incomplete, because the operator is the third
party most likely to be careless. In production, no human has direct access to:

- query strings (they do not exist)
- result payloads beyond the cache TTL
- per-user anything (there is no per-user anything)
- Prometheus labels containing query text (the label set has no such field)

Operators see aggregate crawl statistics, index health, latency, and error rates. Debugging a
user-reported bad result works because the user can see the plan themselves and share it —
which is the whole reason the plan is serialisable.

## Review triggers

| Trigger | Action |
| ------- | ------ |
| A change to the request path | Update [privacy-model.md](./privacy-model.md) in the same pull request |
| A new persistence layer | Update [data-flow.md](./data-flow.md) and add a deletion procedure |
| A new outbound request | Justify against the egress allowlist, or remove it |
| A new model integration | Re-verify the no-external-API constraint |
| Quarterly | Full review of all three documents against the code |

## Related

- [../threat-model.md](../threat-model.md) — adversaries, trust boundaries, mitigations
- [../security/](./) — implementation of the security properties
- [PRIVACY.md](../../PRIVACY.md) — the user-facing statement
- [../adr/](../adr/) — decisions with privacy consequences are recorded as ADRs