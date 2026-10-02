# Query planning

After parsing ([query-parsing.md](./query-parsing.md)) comes planning: deciding what kind of
question this is, which language it is in, and which corrections and expansions are
warranted. All of it is pure computation on the query — no I/O, no user state, nothing
retained.

## Intent classification

Six classes, from term patterns and operators. Simple and inspectable, because a wrong
intent multiplies weights and a black-box classifier would make that unexplainable.

| Class | Triggers | Effect |
| ----- | -------- | ------ |
| `informational` | default, wh-questions, "what is", "how to" | `w_context` normal |
| `navigational` | a host-like term, `site:`, a bare domain, a filename | `w_context` × 1.5, prefer canonical, boost the domain |
| `transactional` | "buy", "price", "download", "sign up", "login" | freshness intent, penalise soft-404 and download-only |
| `definitional` | "definition", "meaning of", "what is" | boost definitions, prefer reference content type |
| `troubleshooting` | "error", "fails", "not working", "exception", a quoted error string | boost documentation, penalise Q&A older than the version mentioned |
| `time_sensitive` | see [../ranking/freshness.md](../ranking/freshness.md) | freshness weight × 3, decay floor applied |

```text
intent = highest-scoring class
score  = Σ (pattern_weight × matched_pattern_weight)
tie    → 'informational'   (the default is always the safe answer)
```

A navigational intent deserves special mention: when the query looks like a URL or a host,
the right answer is that page. It gets a `w_context` boost, canonical preference, and a
domain match bonus. It does not get a redirect or a special-case "did you mean" — the ranking
signals handle it, so the behaviour is explainable by the same rules as everything else.

## Language detection

```text
1  explicit lang: operator wins, always
2  Unicode script of the query (CJK, Cyrillic, Arabic, …) → strong signal
3  stopword-frequency ratio across a per-language profile
4  a lightweight statistical detector for short queries
5  fall back to the interface language of the request
```

Accuracy matters more than usual here, because a wrong language setting silently degrades
everything downstream: stemming, stopwords, spelling, and the index partition being searched.
That is why step 3 uses per-language profiles — a query of mostly stopwords in one language
is the strongest cheap signal available.

The language also selects the shard set, so a wrong decision returns nothing rather than
something poor. Failures are therefore visible and cheap: they look like zero results, which
is diagnosable, instead of subtly degraded results.

## Stopwords

```text
stop words are kept in the index
stop words are assigned weight 0.15 rather than being removed
```

Kept, weighted, never deleted. Standard stop-word lists throw away "how", "why", "not", and
"vs", all of which carry intent. Weighting them low preserves the option and avoids the
failure mode where a stop-word-only query returns nothing.

The same applies to the index: stop words are indexed, so `the` alone retrieves documents.

## Spelling correction

```text
1  if the query returns ≥ threshold results → no correction   (never correct a good query)
2  if every term has a suggestion:
       correction applied, marked, shown as "showing results for X"
       original preserved as an OR alternative
3  if only some terms have suggestions:
       correct those into the must clause, keep the originals as should
4  if nothing suggests but results are empty:
       one token dropped, one retry, bounded to a single retry
```

Three rules that matter more than the algorithm:

- **Never correct when results already exist.** Correcting a working query is worse than any
  miss.
- **Always keep the original.** The original enters as a disjunct, so a wrong correction
  costs ranking position, not all results.
- **One retry only.** Spelling loops are a classic latency and confusion bug. One retry, then
  the honest empty state.

Implementation is trigram similarity over the term dictionary with an edit-distance ≤ 2 guard,
plus a curated confusable-pair list (`form`/`from`, `there`/`their`, `affect`/`effect`). The
curated list is the part that actually performs; the trigram machinery is the safety net for
typos nobody anticipated.

## Expansion

```text
inflection   simple morphology, not stemming to the root   run/runs/running/running → run
synonym      curated, only for genuinely asymmetric pairs   "close" ≠ "shut"
acronym      US → United States, HTML → HyperText…
```

All expansions are weighted `0.4` relative to the original and remain tagged in the plan, so
the explain output shows exactly what was expanded. Expansion does not run for queries with
`intitle:` or explicit phrases — both are precise user intent, and expanding them would
override an explicit choice.

## Time sensitivity

Detailed in [../ranking/freshness.md](../ranking/freshness.md). At planning time the job is
only to classify and to parse `before:`/`after:`/`freshness:` into concrete date bounds.

```text
strong:  latest, current, today, this week, 2026, now, release, version, price
weak:    recent, new, latest version
none:    everything else, including historical questions about recent-looking words
```

A historical question must not be captured by a bare year or the word "current". A query like
"current state of the art in 2019" is historical, and the plan detects the four-digit year
outside the recency window and drops the freshness intent. This is the main false-positive
mode for time-sensitive detection and it is worth a regression test of its own.

## The plan

```rust
struct QueryPlan {
    terms: Terms,                    // parsed clauses and operators
    intent: IntentClass,
    language: LanguageTag,
    confidence: f32,                 // 0…1, surfaces "unusual spelling" hints
    corrections: Vec<Correction>,
    expansions: Vec<Expansion>,
    time_sensitive: TimeSensitivity, // None | Weak | Strong, with bounds
    freshness_intent: bool,
    shard_hints: Vec<ShardId>,       // language + optional site narrowing
}
```

The plan is the complete input to retrieval and ranking, is serialisable for
[../ranking/explainability.md](../ranking/explainability.md), and contains no field that
identifies a user or a session.

## Testing

- Intent classification: one labelled example set per class, plus the known-overlap cases.
- Language detection: a per-language fixture set including short queries and code-switched
  queries, where the correct behaviour is a lower confidence rather than a wrong answer.
- Correction: the "never correct a good query" property asserted directly; the single-retry
  bound asserted by counting retrievals.
- Expansion: no expansion when `intitle:` or a phrase is present.
- Time sensitivity: the historical-question regression above.
- Property tests: planning is deterministic; a plan for an empty query is well-formed;
  every weight is within its declared range.