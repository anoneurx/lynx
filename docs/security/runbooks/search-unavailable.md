# RB-01 · Search returns nothing

**Severity:** SEV1 if all results fail, SEV2 if partial · **Owner:** operations

## Symptom

Zero results for queries that should match, or a search page with an empty result set across
many unrelated queries.

## First action

```bash
lynx-status --json | jq '{generation, shards: [.shards[] | {id, state, doc_count}]}'
```

## Diagnosis

```text
1  all shards missing or closed?      → generation problem, go to §1
2  one shard failing?                 → isolated, go to §2
3  all shards healthy, zero results?  → query or index-content problem, go to §3
```

### §1 Generation problem

```bash
lynx-index status --generation latest
lynx-index verify --generation <gen> --manifest   # manifest verification
```

| Finding | Cause | Action |
| ------- | ----- | ------ |
| Latest generation missing | Build failed | Roll back to the previous generation |
| Manifest mismatch | Index corruption | SEV1, [RB-03](index-corruption.md) |
| Generation present but not committed | Build interrupted | Commit or roll back |
| All shards closed | Reader exhausted or deploy error | Restart the search process; readers are read-only and safe to reopen |

### §2 Single shard failing

| Finding | Cause | Action |
| ------- | ----- | ------ |
| State `degraded` | Recent reader error | Leave it; it recovers on the next successful query |
| State `unavailable` | Repeated failure | Roll back the whole generation; do not wait |
| `doc_count` far below the manifest | Partial index | Roll back |
| Lock held on the shard | Stale lock from a killed build | Remove the lock after confirming no build is running |

### §3 Index healthy, results empty

```bash
lynx-query explain --query "<failing query>" --generation <gen> | jq '.query_plan, .candidates_retrieved'
```

| Finding | Cause | Action |
| ------- | ----- | ------ |
| `candidates_retrieved = 0` | Corpus genuinely lacks the content | Not an incident; verify with a known-present query |
| All terms dropped as stopwords, score 0 | Stopword weights misconfigured | Restore the previous config version |
| Query parsed to nothing | Parser regression | Roll back; check the last parser deploy |
| Known-good query also returns 0 | Index or retrieval regression | Roll back, then [RB-03](index-corruption.md) |

The discipline: **verify with a control query before declaring it not an incident.** "I would
expect results" is a hypothesis; a query known to be in the index is a control.

## Mitigation

| Situation | Action | Reversible |
| --------- | ------ | ---------- |
| Bad generation | Roll back to the previous | Yes |
| Bad config version | Roll back the config | Yes |
| Parser regression | Roll back the search image | Yes |
| Corrupt index | Serve from the previous generation, rebuild forward | Yes |

Every mitigation here is a rollback. There is no fix-forward option in this runbook, because the
cause is unknown until after service is restored.

## Verification

```bash
# 1. a known-present query returns results
lynx-query explain --query "test control query" | jq '.results | length'

# 2. the golden query set passes
lynx-eval ranking --golden --generation <rolled-back-gen>

# 3. every shard reports healthy
lynx-status --json | jq '[.shards[] | select(.state != "healthy")] | length'   # expect 0
```

Do not close until the golden set passes. Restored availability with a wrong index is a SEV1 in
disguise.

## Escalate

- Manifest verification failed → SEV1, [RB-03](index-corruption.md)
- Corruption recurs after rollback → SEV1, engineering investigation before any new build
- Cause is a data loss event → SEV1 plus the privacy lead