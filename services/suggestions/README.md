# `services/suggestions` — query suggestions

**Deployment:** v0.1 — library inside `apps/api`, reading a dedicated read-only replica of
the term/document-frequency table (or an n-gram trie built at index commit).

## Design

Suggestions are derived entirely from the corpus, never from user queries. This is both a
privacy requirement and a quality requirement: user-query-derived suggestions would be
exactly the profiling we refuse to build.

```text
candidate sources (all corpus-derived)
  ├── term prefixes          popular indexed terms
  ├── document title prefixes
  ├── n-gram continuation    for mid-word completion
  └── popular completions    static frequency table with manual editorial control

  → popularity score (df × length prior × freshness prior)
  → blocklist filter        (policy, safety, and brand-safety term lists)
  → language filter          (user's Accept-Language, which is not stored)
  → top N
```

## Why not "popular searches"

A "people also search for" widget built from LYNX query logs would be the exact product
decision this project refuses to make. If LYNX ever ships such a widget, it must be built
from corpus statistics, and the reasoning has to be in the PR.

## Behaviour

- Endpoint returns suggestions as the user types, debounced client-side (≥ 120 ms).
- Cached in Redis for 5–15 minutes keyed by prefix hash; no query text in the key.
- Same prefix-shaping privacy rule as search: the suggest response is `no-store` and the
  prefix is never logged.
- Empty/short prefixes (1 character) return nothing, to avoid serving a frequency
  oracle over the whole term dictionary.
- Response capped at 10 items.

## Files

```text
services/suggestions/src/
├── lib.rs          trait SuggestionSource { fn suggest(&self, prefix, limit) }
├── corpus.rs       term/title prefix provider
├── ngram.rs        continuation trie provider
├── ranking.rs      popularity + language + blocklist filtering
└── blocklist.rs    policy term lists (checked in, versioned, no user data)
```