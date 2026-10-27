# Limitations

The honest list of what this engine cannot do, written down so nobody has to
discover it in production. The design keeps *one* copy of each fact; where a
limitation is documented elsewhere (spec risks, [quality evaluation](../testing/quality-evaluation.md)),
this page links rather than duplicates.

## Ranking quality

- **No click or engagement data.** LYNX logs nothing about what any user reads, so
  ranking is corpus-statistics + editorial signals + a judged set. The judged set
  bounds what can be *verified*: gates measure NDCG/MRR/Recall against **labels
  that exist**, and quality outside the judge set's topics is not guaranteed by
  any number in CI.
- **Freshness is bounded by the crawl.** A page's published/modified dates are what
  each site self-reports the last time the crawler visited. A "stale" result can be
  caused by a misreported date upstream; LYNX cannot detect this without
  re-crawling, and it will not fake it.
- **Authority is inherited problematically.** Host authority is a proxy
  (crawling-trackable) for a real-world social graph LYNX does not have. Known
  consequences are documented in [ranking/authority](../ranking/authority.md).

## Retrieval coverage

- **A phrase the corpus never had cannot surface.** Retrieval is exact over the
  indexed text with the tokenizer in force at generation time. The measured Recall
  gaps in the quality harness (e.g. vocabulary the judgment expects but no document
  carries) are real, recorded, and not "fixed" by editing labels — see the gate
  philosophy in [quality-evaluation](../testing/quality-evaluation.md).
- **Corpus drift is silent until measured.** The index only knows what the crawl
  reached; a whole-site outage in the crawl plane looks exactly like the web went
  away.

## Privacy-bound product limits

- **No personalization, no sitting history, no query completion trending.** The
  suggestions engine works from corpus terms and titles only
  ([suggestions](../search/suggestions.md)) and **cannot** serve trending or
  personalized completions, which is a known, deliberate trade.
- **Anonymous rate limiting is coarse.** NAT/VPN users share a prefix bucket; the
  [privacy model](./privacy-model.md) treats correlation attacks as out of scope.

## Operational posture

- **Backups run on schedules; restores run on drills.** Both are honest only in
  rehearsal (see [backup](../operations/backup.md)); skip the drill and the plan
  decays silently.
- **Observability is deliberately narrow.** There is no per-request forensic
  trail beyond shapes and aggregates; when an incident asks "what did this visitor
  search?", the answer is "nothing recorded that". See
  [telemetry](./telemetry.md).
