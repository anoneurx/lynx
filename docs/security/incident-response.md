# Incident response

What to do when something is wrong, in the order the steps appear. The runbooks assume the
reader is under pressure and unfamiliar with the system, because that is when they will be
reading.

## Severity

| Level | Definition | Response | Update cadence |
| ----- | ---------- | -------- | --------------- |
| **SEV1** | Data exposure, integrity compromise, or the index is serving wrong results at scale | Immediate, all hands | Every 30 min |
| **SEV2** | Major degradation: unavailable, or severely degraded quality | Within 15 min | Hourly |
| **SEV3** | Partial degradation, or a component down without user-visible impact | Business hours | Daily |
| **SEV4** | Cosmetic, or a bug with no data or availability impact | Normal queue | On resolution |

Ranking integrity failures are SEV1 regardless of blast radius. A search engine returning
confidently wrong results is worse than one returning an error, and "only 2 % of results are
wrong" is not a mitigation.

## Roles

| Role | Responsibility |
| ---- | -------------- |
| Incident commander | Coordination, decisions, the clock. Does not debug |
| Operations lead | Diagnosis and mitigation |
| Communications lead | Status page, user communication, disclosure |
| Scribe | Timeline, decisions, evidence. The most understaffed role |
| Privacy lead | Required on any incident involving data handling |

One person holds incident command. Multiple people debugging is fine; multiple people deciding is
how response times double.

## Response procedure

```text
1  DETECT      alert fires, or a report arrives
2  CLASSIFY    severity, then scope
3  CONTAIN     stop the bleeding; prefer a reversible action
4  MITIGATE    fix or roll back
5  ERADICATE   remove the cause
6  RECOVER     restore service, verify with an invariant check
7  REVIEW      blameless postmortem within 5 working days
```

Contain before diagnose. A rollback that may not fix the problem is still the right first move
when the alternative is users seeing the problem for another hour.

## Runbook index

| Symptom | Runbook | First action |
| ------- | ------- | ------------ |
| Search returns nothing | [RB-01](runbooks/search-unavailable.md) | Check shard health, then the generation |
| Search latency spike | [RB-02](runbooks/latency-spike.md) | Identify the slow stage from spans |
| Index appears wrong | [RB-03](runbooks/index-corruption.md) | Freeze the generation, verify against the manifest |
| Crawler stopped | [RB-04](runbooks/crawler-stalled.md) | Check the frontier backlog and worker health |
| Suspected SSRF or scanner traffic | [RB-05](runbooks/ssrf-attempt.md) | Block at the edge, then review the chain |
| Suspected prompt injection | [RB-06](runbooks/prompt-injection.md) | Disable Astra, keep search serving |
| Suspected secret exposure | [RB-07](runbooks/secret-exposure.md) | Rotate first, investigate second |
| Cache behaving unexpectedly | [RB-08](runbooks/cache-failure.md) | Bypass, then diagnose |
| Data may have been exposed | [RB-09](runbooks/data-exposure.md) | Contain, then assess what exists |
| Index bloat or disk pressure | [RB-10](runbooks/disk-pressure.md) | Reclaim, then throttle ingestion |

Rotate before investigating on any suspected secret exposure. A leak that is still live while
being investigated is still live.

## Communications

| Stage | Content | Audience |
| ----- | ------- | -------- |
| First notice | What is affected, what is not, when the next update comes | Status page, in under 30 min |
| Ongoing | Current state, mitigation applied, next update time | Status page |
| Resolution | What happened, what was affected, what changed | Status page, changelog |
| Disclosure | Technical detail, affected versions, detection guidance | Advisory, security mailing list |

The first notice does not need a cause. It needs honesty about what is affected and a commitment
to the next update time. Holding a notice until the cause is known is the most common way an
incident becomes a trust incident.

## Postmortem

```text
within 5 working days, blameless, published internally
```

| Section | Content |
| ------- | ------- |
| Timeline | Detection, diagnosis, containment, recovery — with timestamps |
| Impact | Users affected, duration, data involved |
| Root cause | The technical chain, not a person |
| Contributing factors | The conditions that allowed it |
| What went well | Kept, so it is not "fixed" away |
| Action items | Each with an owner and a due date, tracked to completion |
| Invariant check | What assertion would have caught it earlier, and where it now lives |

"Action item: be more careful" is not an action item. Every item is either a code change, a
gate, an alert, or a documented procedure. A postmortem whose items are all intentions has
learned nothing.

## Invariants

Invariants are assertions that must always hold, checked continuously. They are the difference
between "we noticed" and "we knew".

| Invariant | Alert on violation |
| --------- | ----------------- |
| No query text in any persistent store | Any occurrence → SEV1 |
| No index generation is served that fails manifest verification | Any failure → SEV1 |
| Every result's score equals its computed formula | Any divergence → SEV1 |
| No SSRF-blocked request ever reached a socket | Any occurrence → SEV1 |
| Egress contains only allowlisted destinations | Any other destination → SEV1 |
| No plaintext secret in a backup | Any occurrence → SEV1 |
| Index health: generation age, doc count, error rate | Threshold breach → SEV2 |
| Latency p95 | Budget breach → SEV2 |
| Crawl rate per domain | Politeness breach → SEV3 |
| Tombstone verification pass rate | Any failure → SEV1 for the affected document |

The top row deserves emphasis. Because LYNX does not log queries, the check that no query text
exists anywhere is not a routine assertion — it is the mechanism that verifies the central
architectural claim. If it fires, either the architecture changed or the checker is broken, and
both are SEV1.

## Post-incident

| Action | Deadline |
| ------ | -------- |
| Postmortem published | 5 working days |
| Action items assigned | At the postmortem |
| Action items complete | 30 days, or explicitly deferred with a reason |
| Advisory published for anything user-visible | 90 days, or immediately if actively exploited |
| Invariant added for any missing detection | With the postmortem |

The last row is the only one that compounds. Every incident that was detected by a human rather
than an alert should result in an alert, because the next occurrence will be at 3 a.m. and the
human will not be there.

## Related

- [../threat-model.md](../threat-model.md) — the threats these runbooks respond to
- [../operations/](../operations/) — deployment, rollback, monitoring
- [../security/key-management.md](./key-management.md) — the rotation runbooks referenced by RB-07
- [SECURITY.md](../../SECURITY.md) — reporting a vulnerability