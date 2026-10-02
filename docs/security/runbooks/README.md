# Runbooks

Operational runbooks referenced from [../incident-response.md](../incident-response.md). Each is
written to be followed by someone who has not read the codebase, because that is who will be
reading at 3 a.m.

| ID | Symptom | Severity | Runbook |
| -- | ------- | -------- | ------- |
| RB-01 | Search returns nothing | SEV1–2 | [search-unavailable.md](./search-unavailable.md) |
| RB-02 | Latency spike | SEV2 | [latency-spike.md](./latency-spike.md) |
| RB-03 | Index corruption or wrong results | SEV1 | [index-corruption.md](./index-corruption.md) |
| RB-04 | Crawler stalled | SEV2–3 | [crawler-stalled.md](./crawler-stalled.md) |
| RB-05 | Suspected SSRF or scanner traffic | SEV1 | [ssrf-attempt.md](./ssrf-attempt.md) |
| RB-06 | Suspected prompt injection | SEV2 | [prompt-injection.md](./prompt-injection.md) |
| RB-07 | Suspected secret exposure | SEV1 | [secret-exposure.md](./secret-exposure.md) |
| RB-08 | Cache failure | SEV3 | [cache-failure.md](./cache-failure.md) |
| RB-09 | Suspected data exposure | SEV1 | [data-exposure.md](./data-exposure.md) |
| RB-10 | Disk pressure | SEV2 | [disk-pressure.md](./disk-pressure.md) |

## Common structure

```text
symptom        what you observe, in the words an alert or a user uses
severity       and why that severity, including when it escalates
first action   the one command to run before reading further
diagnosis      a decision tree, with what to look for at each branch
mitigation     ordered by reversibility and cost
verification   what must pass before the incident is closed
escalate       the conditions, and to whom
```

The ordering rule — **contain before diagnose, cheap before expensive, reversible before
permanent** — is what makes these usable without judgement under pressure.

## Principles

1. **Every runbook starts with a command**, not with explanation. Under pressure, prose is
   slower than a command.
2. **Verification is mandatory.** Closing without the verification section is how an incident
   becomes a second incident.
3. **Mitigations are reversible.** A rollback is preferred to a fix, because a fix made under
   pressure is a fix that will be undone.
4. **The last resort is stated.** Every runbook says which action is the one to avoid, because
   that is the one someone will reach for under stress.
5. **No runbook evades a block.** Rotating identity to get past a ban, disabling politeness to
   recover throughput, or bypassing privacy checks to restore latency are explicitly rejected,
   usually in their own row.
6. **Every incident adds an invariant.** The postmortem is not complete without a new check.

## Maintenance

| Review | Frequency |
| ------ | --------- |
| Each runbook after it is used | Within 5 days of the incident |
| Each runbook against the current system | Quarterly |
| After any architectural change | Immediately |
| Drill: walk through RB-01, RB-03, and RB-07 | Quarterly, with someone who did not write them |

The drill is the part that finds out whether a runbook is actually usable. A runbook written by
the person who knows the system will read as complete whether or not it is, and the only way to
learn which it is to hand it to someone else.

## Adding a runbook

A runbook is added when an alert exists that has no documented response. If a symptom can page
someone and there is no runbook for it, that is a gap in the runbook set, not in the responder.

New runbooks follow the common structure above and must include a verification section, a
last-resort statement, and the escalation conditions. A runbook without escalation conditions
tends to become a place where incidents stop, rather than escalate to.