# Tokenization and text analysis

Query-time and index-time analysis must agree exactly. Anything that differs is a query
that silently returns nothing, which is the worst kind of bug in a search engine: no error,
just missing results.

## Pipeline

```text
raw text
 → sanitise (control, zero-width, bidi)
 → NFC normalisation
 → width folding (full-width → half-width)
 → case folding (locale-independent)
 → segmentation (Unicode word boundaries, language-aware)
 → stopword marking (not deletion)
 → stemming (index-time and query-time, same algorithm, same version)
 → n-gram generation for CJK
```

## Segmentation

Unicode text segmentation (UAX #29 word boundaries) rather than whitespace splitting, so
that:

- Scripts without spaces (Thai, Khmer, Lao) segment correctly via the Unicode dictionary
  break rules.
- CJK ideographs are identified and routed to bigram generation instead of being treated as
  words.
- Emojis, combining marks, and zero-width joiners do not produce phantom tokens.
- Apostrophes and hyphens are handled per locale rules, so `don't` and `co-op` behave
  sensibly.

Punctuation is a delimiter, not content. Numbers are tokens: LYNX indexes numeric tokens so
that version numbers (`1.2.3`), years, and quantities are findable, with a small set of
pure-number stopwords removed only in the lowest-weight field.

## Stopwords

```text
English   a, an, and, are, as, at, be, by, for, from, has, in, is, it, of, on,
          or, that, the, to, was, were, will, with, …
German    der, die, das, und, ist, von, zu, mit, …
(and per-language equivalents for every supported language)
```

Stopword handling in LYNX is deliberately unusual:

- Stopped tokens are **indexed in a dedicated low-weight field**, not deleted.
- A query stopword does not remove the clause; it removes the *scoring* contribution while
  still requiring the term to be present (`must` becomes a filter on a low-weight field).
- The practical effect: `the cat` matches `the cat sat`, and `to be or not to be` is a valid
  phrase query. Neither works in a conventional stopword-deleting index.

Consequence to be honest about: index size is larger than a conventional design. Measured,
and accepted in exchange for correctness.

## Stemming

```text
English      Porter2     (not Porter1 — fewer false conflations)
German       Snowball
French       Snowball
Spanish      Snowball
Italian      Snowball
Portuguese   Snowball
Russian      Snowball
Dutch        Snowball
Swedish      Snowball
Danish       Snowball
Norwegian    Snowball
Turkish      light suffix stripping (agglutination breaks naive Stemmer)
Finnish      none (long suffixes, high damage)
CJK          none (bigrams)
unsupported  none
```

Rules:

- **Index-time only.** The stored term is the stem. Queries are stemmed with the same
  algorithm.
- The algorithm and version live in the index manifest. Both sides read it. A mismatch is a
  test failure, not a production possibility.
- Stemming is applied per language from the document's language, so an English stemmer never
  touches Russian text.
- Aggressive stemmers are not used. Over-stemming merges unrelated words, and the cost shows
  up as mysteriously bad results rather than as an error.
- Original surface forms are preserved in the dictionary for display, so a stemmed result
  shows the original token in the snippet.

## CJK

No word boundaries, so LYNX uses **character bigrams**:

```text
日本語の検索  →  日本語, 本語, 語の, の検, 検索
```

- Bigrams are indexed in the same weight field as word tokens.
- Single characters are not indexed as terms; they carry too little signal and bloat the
  dictionary.
- Query-side: a CJK query is bigram-expanded, and the number of bigrams is capped
  (`max_expansions`, default 64) so a long CJK query cannot explode.
- For Japanese, kanji-only queries work well with bigrams; kana-only queries are noisier,
  which is a known quality limitation rather than a solved problem.

## Query-time analysis

The query pipeline applies the identical chain, plus:

```text
after tokenisation
 → spell correction (edit distance ≤ 2, tokens ≥ 5 chars, dictionary-gated)
 → morphological expansion (inflectional variants, capped)
 → curated synonym expansion (small, hand-maintained, capped)
 → stopword down-weighting
 → total term cap (max_expansions, default 64)
```

Caps exist so that a query cannot blow up the cost of a single request:

| Guard | Value |
| ----- | ----- |
| Query byte length | 512 |
| Token count | 32 |
| Expansion cap | 64 terms |
| Fuzzy candidates per term | 5 |
| Phrase terms | 16 |
| Boolean nesting depth | 8 |

When a cap is hit, the query still executes — on the terms it had. Failing a search because
a correction would be too expensive is worse than a slightly less corrected search.

## Dictionary and vocabulary

Terms live in the index dictionary, which is the vocabulary the spell corrector and
suggestions use.

- Vocabulary is derived **only from the crawled corpus**. No external word lists, no user
  query logs. This keeps suggestions corpus-derived and privacy-clean by construction.
- The corrector uses edit distance with a frequency threshold, so a rare-but-real term is
  corrected only when the correction is confident.
- Dictionary size is a documented cost of the privacy decision: no user-derived vocabulary
  means slightly worse correction than an engine that watches what people type.

## Versioning

```text
tokenizer_version = 3
includes          = segmentation, stopwords, stemming, ngram policy
changed           → full index rebuild (new generation)
```

A version bump is an index-generation change, and results are attributed to the version they
were produced with. A user who reports a bad result can be told which analysis version
produced it.

## Testing

- **Property tests:** normalisation is idempotent; `analyse(analyse(x)) == analyse(x)`;
  the tokenizer never panics or allocates unboundedly on arbitrary input.
- **Fuzzing:** `fuzz_tokenize` over arbitrary UTF-8 including invalid sequences, mixed
  scripts, and combining-mark floods.
- **Equivalence test:** the query analyzer and index analyzer are the same code path, invoked
  with the same version, and a test asserts index-time and query-time output agree for a
  corpus of terms.
- **Golden tests:** a checked-in multilingual corpus with expected token streams, reviewed
  when the tokenizer changes.
- **Coverage requirement:** every supported language has a stemmer test and a stopword test.