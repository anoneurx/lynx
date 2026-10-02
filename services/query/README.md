# `services/query` — query processor

**Deployment:** v0.1 — library inside `apps/api`. CPU-bound and stateless, so it scales by
adding API replicas. Full design: [docs/search/](../../docs/search/README.md).

## Responsibilities

```text
raw query string
  → length / token budget check            (reject early, cheap)
  → lex                                    (unicode-aware, operator-aware)
  → parse                                  (grammar in query-language.md → QueryAst)
  → validate                               (operator arity, field names, quoted forms)
  → normalise                              (case, unicode NFKC, width, diacritics policy)
  → tokenise per field                     (same tokenizers as the indexer)
  → spell correct                          (edit distance ≤ 2 on long tokens only)
  → expand                                 (small curated synonym set + morphological variants)
  → compile                                → QueryPlan for the index reader
```

## QueryAst

```text
QueryAst
├── Bool
│   ├── must      : Vec<QueryAst>
│   ├── should    : Vec<QueryAst>
│   ├── must_not  : Vec<QueryAst>
│   └── filters   : Vec<FieldFilter>      (site:, filetype:, lang:, date range)
├── Term      { text, field, boost, fuzzy }
├── Phrase    { terms, slop, field }
├── Prefix    { prefix, field }
├── Range     { field, from, to }         (date filters)
└── All       {}                          (empty query / match-all, guarded)
```

`All` is only constructible from an explicit internal path — a user query must never
compile to match-all, which would be an index-dump vector.

## Design rules

- **No user-supplied regex.** The query language has no regex operator in v0.1. This is a
  denial-of-service decision, not a missing feature.
- **No unbounded expansion.** Fuzzy, synonyms, and morphological variants are capped by
  config (`max_expansions`, default 64 terms). An unexpandable query still executes as a
  plain term query — it never fails because expansion failed.
- **Deterministic.** The same query string with the same config produces the same plan.
  No randomness, no time dependence in parsing.
- **Fail closed, cheaply.** Malformed operators are rejected with
  `UNSUPPORTED_OPERATOR` / `INVALID_QUERY`, never silently reinterpreted. Silently
  dropping a `-term` would be worse than an error.
- **The plan is inspectable.** The compiled `QueryPlan` is a serialisable struct so it can
  be logged in *shape* (operator counts, term counts) and shown in the admin console
  during development. Raw terms are only ever serialised into the plan when
  `debug.explain_plan = true` and the request is authenticated as an operator.