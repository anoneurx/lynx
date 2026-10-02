# Query parsing

Parsing turns a raw string into a `QueryPlan`. It is the only place where the user's
typographic intent is interpreted, so the rules are explicit and the parsed form is
inspectable.

## Operators

| Operator | Effect | Example |
| -------- | ------ | ------- |
| `"…"` | exact phrase, slop 0 | `"exact phrase"` |
| `-term` | exclude | `rust -golang` |
| `site:` | restrict to host or registrable domain | `site:docs.rs` |
| `inurl:` | substring match on the URL | `inurl:/api/v1` |
| `intitle:` | require the term in the title | `intitle:changelog` |
| `ext:` | restrict by file extension | `ext:pdf` |
| `before:` / `after:` | date filter, `YYYY-MM-DD` or `YYYY` or `YYYY-MM` | `after:2025-01` |
| `freshness:` | recency window | `freshness:week` |
| `filetype:` | MIME type | `filetype:pdf` |
| `lang:` | language filter | `lang:de` |
| `OR` / `or` | disjunction between terms | `rust OR zig` |
| `+term` | force-include, overrides stopword handling | `+the` |

```text
rust site:docs.rs intitle:changelog after:2025-01 -github "async runtime"
```

```yaml
must:      [rust]
should:    []
must_not:  [github]
phrase:    ["async runtime"]
filters:
  site:     docs.rs
  intitle:  changelog
  after:    2025-01-01
```

## Grammar

Precedence is fixed and documented:

```text
1  operators        parsed anywhere in the string, on token boundaries
2  phrases          "…" — content inside is never treated as an operator
3  OR               splits the surrounding term group into disjuncts
4  negation         -term and NOT term, collected into must_not
5  default          all remaining terms are must (all-must, see below)
```

**All-must is the default.** `rust async runtime` requires all three terms. This is correct
for a small index: a document matching more of the query is genuinely more relevant, and
loose OR semantics in a small corpus mostly surfaces weakly-related noise. It also makes
`OR` meaningful, because the user has to ask for it.

## Parsing rules that matter

| Rule | Behaviour | Why |
| ---- | --------- | --- |
| Operators only on token boundaries | `site:example.com` parses; `mysite:foo` does not | Avoids mangling URLs and times in queries |
| Escape with `\` | `site\:example` is the literal token | The only escape mechanism, deliberately simple |
| `"` inside a phrase is literal | `"say \"hi\""` | No nested quoting |
| Unbalanced `"` | Treated as a literal quote character | Never errors on the user |
| `OR` is a token, not a substring | `ORDINARY` is one term | Case-insensitive match on the whole token |
| Negation requires a term | A trailing `-` is literal | No silent drop |
| `:` with empty value | Operator is ignored, token kept literally | `site:` alone is a typo, not a filter |
| Repeated operator | Last occurrence wins | Deterministic, matches user intent on correction |
| Leading `-` on a number | Treated as a term, not negation | `before:-5` is nonsense; make it literal |

## The parsed form

```rust
struct QueryPlan {
    raw_len: u16,                       // for the privacy-bounded length check
    must: Vec<Term>,
    should: Vec<Term>,
    must_not: Vec<Term>,
    phrases: Vec<Phrase>,               // each with optional slop
    disjunction: bool,
    filters: Filters,
    operators_used: Vec<Operator>,
}

struct Filters {
    site: Option<Domain>,
    inurl: Option<String>,
    intitle: Vec<String>,
    ext: Vec<String>,
    filetype: Vec<MimeType>,
    lang: Option<LanguageTag>,
    before: Option<Date>,
    after: Option<Date>,
    freshness: Option<FreshnessWindow>,
}

enum Term {
    Exact(String),
    Prefix(String),        // only in suggestions, never in results
    Fuzzy(String),         // edit-distance ≤ 1–2, dictionary-gated
    Expanded { original: String, expansion: String, weight: f32 },
    Corrected { original: String, corrected: String, weight: f32 },
}
```

`Operators` is a typed list, not a set of booleans. It exists for two reasons: the plan is
serialisable for `/explain`, and every operator must have a documented behaviour somewhere,
so the enumeration is the checklist.

## Filters vs scoring

Two filters are **hard** (results that do not match are excluded):

- `site:`, `inurl:`, `ext:`, `filetype:`, `lang:`, `before:`, `after:`, `freshness:`

Everything else contributes to the score rather than filtering:

- `intitle:` → a large term weight, not a filter. A page with the term in the title that does
  not contain it in the body is still relevant.
- `before:`/`after:` on a document with **no date** → excluded, because a date filter is an
  explicit request and a document that cannot satisfy it should not appear. The no-results
  state says this explicitly, because it is the most surprising behaviour in the parser.
- Negative terms → a scoring penalty, not exclusion. `-github` means "downrank pages about
  github", and a page that is overwhelmingly about the term you asked for still deserves to
  appear. Pure exclusion would let `-rust` empty a `rust` query.

That distinction is a design decision, not an oversight. Hard filters express constraints;
penalties express preferences.

## Validation and rejection

| Condition | Behaviour |
| --------- | --------- |
| Empty after parsing | Empty result state, never an error |
| Longer than 512 bytes | Truncate at a term boundary, attach a visible notice |
| `site:` with an invalid domain | Operator ignored, token literal |
| `before:` after `after:` | Both applied; empty range → empty result, stated plainly |
| More than 64 terms | Truncate with a notice; protects the latency budget |
| Malformed UTF-8 | Replacement characters; cannot cause a panic |

## Escaping

```text
\space      literal space
\"          literal quote
\-          literal hyphen (not negation)
\:          literal colon
\\          literal backslash
\*          wildcard, currently unsupported and returned literally
```

A single escape character, applied before all other parsing. Simple enough to implement
correctly in one pass and to describe in one sentence, which is the bar.

## Testing

- Table-driven tests for the operator grammar: one case per operator, per precedence rule,
  per malformed input.
- Property tests: parsing is idempotent (`parse(format(parse(q))) == parse(q)`); an empty
  input never panics; every operator in the enumeration has a behaviour test.
- A privacy test asserting the plan contains no field beyond those in the struct — no
  incidental capture of the raw string beyond `raw_len`.
- A latency test on 10 000 generated queries asserting p99 parse time stays inside the
  budget; parsing must never be the slow part.