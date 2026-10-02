# Suggestions (autocomplete and related queries)

Typeahead is the highest-leverage privacy decision in a search engine. It is also the feature
most dependent on data LYNX refuses to collect.

The central constraint: **autocomplete cannot be built from query logs, because LYNX has no
query logs.** Everything below is built from the index and the link graph instead.

```text
prefix keystrokes ──▶ prefix index lookup ──▶ frequency prior ──▶ completion
                                                          ▲
                       related queries: co-occurrence in documents, not in queries
```

## Completion

```rust
struct Completion {
    text: String,               // the full suggested string
    matched_prefix: String,
    kind: CompletionKind,       // History | Content | Popular | Related
    prior: f32,                 // 0…1, corpus frequency prior
    doc_count: u32,             // corpus support for this completion
}
```

Three sources, combined:

| Source | How it is produced |
| ------ | ------------------ |
| Corpus prefix | An in-memory prefix dictionary built at index time from term statistics; prefix lookup, O(1) |
| Document titles | A separate title prefix dictionary, so "how to in" suggests a title beginning that way |
| Related terms | Terms co-occurring in the same documents, weighted by BM25F field proximity |

Ranking of completions:

```text
score = 0.40·corpus_frequency_prior
      + 0.25·title_match_bonus
      + 0.20·co_occurrence_prior
      + 0.15·length_prior          (prefer 2–5 words; a 12-word completion is noise)
```

`corpus_frequency_prior` is document frequency from the index. Not query frequency, because
query frequency does not exist. The consequence is real and worth stating: **LYNX's
autocomplete cannot know what people actually type**, only what exists and is linked. It will
not suggest a typo, a meme, or a trending phrase the way a log-driven engine does. That is the
trade, made deliberately.

## Related searches

```text
related(query) = top-k documents matching the query
                 → terms and titles ranked by IDF-weighted co-occurrence in those documents
                 → filtered against the original query to remove tautologies
```

A term is excluded if it is already in the query, if it is a near-synonym of a query term
(producing "rust" → "rustlang"), or if it is a generic co-occurrence artifact
("the", "how", "use"). The IDF weighting is what keeps high-frequency glue words out.

Related searches are a **page-level** feature, computed after results exist, so they inherit
the corpus's bias: LYNX will relate terms the way the indexed web relates them.

## Privacy properties

| Property | Implementation |
| -------- | -------------- |
| No query log | No component writes a query, prefix, or keystroke anywhere. Enforced by having no such write path |
| No per-user state | Completions are identical for every user. No personalisation, no session store |
| No keystroke transmission | Typeahead runs against a prefix index over the corpus; the browser sends the current prefix, which is a request body field and is not retained |
| No third-party autocomplete | The suggestions endpoint is served by LYNX. No third-party script, no keystroke leakage to an ad or analytics vendor |
| No federated learning, no shared counters | Counters are corpus statistics, not user counters. Nothing is transmitted between users |
| Cache keys are hashed plans | [cache.md](./cache.md), keyed by a canonical hash, with a short TTL |

The threat this addresses: keystroke-level data is among the most intimate data a search
engine can collect, and it is the single most common way an engine's privacy posture is
compromised. Building typeahead without logs closes that door rather than promising not to
walk through it.

## Suggestions are deliberately conservative

The most damaging thing an autocomplete can do is suggest something harmful, and a
log-driven engine is structurally poor at avoiding it because it reproduces whatever users
typed, including what they should not have. LYNX's index-derived sources make the failure
mode structurally different: suggestions come from document content, so the filter is
retrieval-shaped rather than a blocklist-shaped patch.

```text
1  content-derived suggestions inherit the index's noindex and spam suppression
2  no user-submitted strings are ever candidates
3  a curated denylist on *suggestion output* as defence in depth — never on the index
4  suggestions are truncated to 4 words, which removes most explicit phrasing
5  no adult, violence, or drug-related completion can be produced, because the
   crawler refused those pages before indexing
```

## Latency and interaction

```text
p50  4 ms   p95  12 ms   (prefix dictionary lookup, in-memory)
debounce: 80 ms in the client
minimum prefix length: 2 characters
maximum suggestions returned: 10
```

The debounce is client-side and fixed at 80 ms. Below 2 characters the suggestion space is
useless and the request cost is pure waste. The endpoint is cacheable and pre-warmed, so a
cold shard does not appear in typeahead latency.

## Accessibility

- `aria-autocomplete="list"`, `aria-activedescendant` for the active option, live-region
  announcements on result count changes.
- Arrow keys move the selection, Enter accepts it, Escape dismisses without changing the
  query.
- Accepting a completion appends it to the input; the browser's undo history is preserved, so
  a wrong suggestion is one keystroke to reverse.
- The typeahead degrades to a non-interactive form with JavaScript disabled; suggestions
  appear as a list below the input.

Accessibility here is not a nice-to-have. A typeahead that only works with a mouse and precise
timing excludes users outright, and the debounce-plus-arrows interaction is a known failure
mode for screen-reader users.

## Testing

- Prefix dictionary correctness: every corpus term is reachable through its prefixes.
- Ranking: a completion with higher corpus frequency wins ties; title matches beat
  non-title matches at equal frequency.
- Privacy: a test asserting the suggestions code path has no write access to any persistence
  layer, enforced at the type level so a new write cannot be added without failing the build.
- Injection: prefixes containing `<`, `"`, and backslashes render as text.
- Co-occurrence quality: a fixture set where the top related terms are the human-chosen ones.
- Latency: a benchmark on 10⁶ dictionary entries asserting p95 stays within budget.