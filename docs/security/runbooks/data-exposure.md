# RB-09 · Suspected data exposure

**Severity:** SEV1 · **Owner:** privacy lead with the incident commander

The first question is not "what happened" but "what exists". For LYNX, the expected answer is
almost nothing, and establishing that quickly is the whole point of the privacy architecture.

## First action

```bash
lynx-verify-privacy
lynx-audit persistence --since 72h      # every write, by target
```

Establish what data was reachable before speculating about how.

## What can exist

| Data | Exists in production? | Exposure assessment |
| ---- | --------------------- | ------------------- |
| Query strings | **No** | There is no store to breach |
| User identifiers | **No** | No accounts, no cookies |
| IP addresses | Transiently, bucketed | Daily-rotating salt; short-lived counters |
| Personal data | **No** | Not collected |
| User agents | Coarse class only | `desktop \| mobile \| bot \| unknown` |
| Crawl state | Yes | Public web data, no personal data |
| Index contents | Yes | Public web content |
| Opt-in ranked results | **No**, off by default | Would be ranked page ids only, never queries |

## Assessment by finding

| Finding | Assessment | Action |
| ------- | ---------- | ------ |
| A persistence write containing a query | SEV1, architecture violated | Contain: disable the write path; then determine how it was added |
| User data found in a store | SEV1 | Contain, assess scope, assess legal obligations |
| Personal data found in the index | SEV1 | Remove it; a home page is personal data in some jurisdictions |
| An opt-in analytics table enabled | SEV2 | Disable immediately; confirm the opt-in was deliberate |
| IP addresses in logs | SEV1 | Disable access logging; rotate the rate-limit salt; scope the window |
| Raw HTML found in any store | SEV1 | Architecture violated; purge |
| Crawl state exposed | SEV3 | It is public web data; the concern is only operational |

The row that matters most is the first: if a query string reached a persistent store, something
was added to the design, and the incident is a design regression as much as a security one.

## Containment

```text
1  stop the write path, not just the reading of it
2  purge the exposed data, and verify the purge
3  preserve the evidence needed for assessment (the artifact, not the copy in the store)
4  determine the exposure window from deployment history, not from memory
5  assess whether the data was reachable outside the system
```

Step 4 is where incidents usually go wrong. The exposure window is the deploy that introduced
the problem, and it must come from the deployment log, because the person handling the incident
is not the person who made the change.

## Assessment

```text
what existed:          the data, precisely
who could reach it:    users of the system, log readers, backup holders, or the public
for how long:          the verified window from the deploy history
what it enables:       the honest answer, even if it is "little"
what is required:      notification, regulator contact, or nothing
```

The pressure in an incident of this type is toward understatement, because over-reporting is
embarrassing and under-reporting is illegal. The guidance is the reverse: state what existed and
who could reach it, and let the assessment follow. An architecture designed to expose nothing
makes an honest "nothing existed" statement a strong position, and it only works if the
statement is true.

## Notification

| Condition | Action |
| --------- | ------ |
| Personal data was reachable by anyone outside the system | Notify affected users and the regulator within the applicable window |
| Personal data was reachable only by trusted operators | Assess per jurisdiction; document the reasoning |
| No personal data existed | No notification required; publish the finding and the fix |
| Cryptographic material was exposed | Credential rotation, but not a data-breach notification |

## Post-incident

```text
1  an invariant that would have caught it, in CI or in lynx-verify-privacy
2  a review step, so the next change is caught before it ships
3  an ADR if the change was intentional — an intentional change to the privacy model is a
   design decision, and an intentional unreviewed one is the failure
```

A privacy incident closed without a new invariant will recur, because the next change is made by
someone who has not read this runbook.

## Escalate

- Any query string in a store → SEV1, privacy lead, engineering lead; treat as a design breach
- Personal data in the index → SEV1, legal review
- Data reachable publicly → SEV1, security lead, and disclosure within the applicable window