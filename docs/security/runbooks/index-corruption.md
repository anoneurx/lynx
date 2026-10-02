# RB-03 · Index corruption or wrong results

**Severity:** SEV1 · **Owner:** engineering, with the incident commander

The most serious class of incident in a search engine: the system works, looks healthy, and
returns wrong answers. It is hard to notice and therefore long-lived, so treat every invocation
as SEV1.

## First action

**Freeze the current generation.** Stop serving it before investigating, or you will be
examining an index that is still changing.

```bash
lynx-index freeze --generation <gen>     # mark, do not deploy
lynx-index status --generation <gen>
```

If the corruption is severe, roll back first and investigate the frozen copy afterwards:

```bash
lynx-index rollback --to <previous-gen>
```

## Verification

Integrity is asserted against the manifest, not by inspection:

```bash
lynx-index verify --generation <gen> --manifest --full
```

| Check | Meaning | Violation means |
| ----- | ------- | --------------- |
| Segment checksum | Bytes are what were written | Storage or write corruption |
| Document count | Matches the manifest | Truncated build |
| Field statistics | Match the corpus statistics | Partial write |
| Term dictionary | Consistent with statistics | Partial commit |
| Tombstone set | No unapplied deletions | Deletion not propagated |
| Sample document round-trip | Parsed fields equal the written fields | Serialisation bug |

## Cause patterns

| Pattern | Cause | Fix |
| ------- | ----- | --- |
| One segment bad, checksum fails | Storage | Restore that segment from backup; investigate the disk |
| Counts match, content wrong | Serialisation bug in the indexer | Roll back, fix, rebuild |
| Tombstones missing | Deletion propagation race | Roll back, fix the propagation |
| Statistics inconsistent | Partial commit | Roll back; enforce commit atomicity |
| Poisoned content, checksums fine | **Index poisoning**, not corruption | See below |

A valid checksum with wrong content means the writer wrote the wrong thing. That is the index
integrity threat from the threat model, and it needs a different response.

## Index poisoning

The attacker shaped the content so that it parsed into fields it should not have, or a
validation step accepted content it should have rejected.

```text
1  identify the affected document ids
2  remove them from every generation, including old ones
3  determine the pattern: which field carried the attacker payload
4  add a validation rule and a regression fixture from the payload
5  rebuild affected shards
6  add an invariant: field-level bounds checked at write time AND at read time
```

Step 6 is the one that matters. Validating only at write time means a bug elsewhere can still
put a bad value in; validating at read time means the serving path is the last line of defence.

## Response

```text
1  freeze or roll back                    stop serving the affected index
2  verify against the manifest            establish what is wrong
3  identify the scope                     one segment, one shard, or the corpus
4  restore from backup if needed           verify after restoring
5  fix the cause                          code, storage, or validation
6  rebuild forward                         with the fix deployed
7  verify the golden set                  ranking must be unchanged for an integrity-only fix
8  write the postmortem                   with an added invariant
```

For an integrity-only fix, the golden ranking set should produce **identical** results. If it
does not, the fix changed behaviour, and that needs its own review before shipping.

## Invariant to add afterwards

Every SEV1 here should close with a new check, because the next occurrence will be quieter:

| Cause | New invariant |
| ----- | ------------- |
| Partial commit | Commit atomicity verified at startup for every generation |
| Serialisation bug | Per-field round-trip check on a sampled document set |
| Missing tombstones | Tombstone set equality between the write log and the index |
| Poisoning | Field bounds checked at read time |
| Storage corruption | Per-segment checksum verified on every reader open, not only at build |

## Escalate

- Any sign of intentional poisoning → SEV1 with the privacy lead, and treat as a security
  incident
- Corruption across multiple shards → halt all index builds; a systemic write bug is more
  likely than simultaneous storage failure
- Recurrence after a fix → stop automatic builds; a debug build with assertions enabled runs
  the pipeline once to reproduce