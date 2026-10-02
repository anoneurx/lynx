# BM25F — the lexical core

Ranking starts with how well a document matches the query *lexically*. Everything after this
is adjustment. Getting this right matters more than any other single decision.

## BM25, briefly

For a single field, BM25 scores a document `d` for query term `t`:

```text
IDF(t) = ln( 1 + (N − df(t) + 0.5) / (df(t) + 0.5) )

        f      qf        f(qf+1)
tf-sat ───────────────────────────── × IDF(t)
        f + k₁(1 − b + b·dl/avgdl) qf
```

- `tf` — term frequency in field `f` of `d`
- `qf` — query term frequency (usually 1)
- `dl` — length of field `f` in `d`
- `avgdl` — average length of field `f` across the corpus
- `k₁ ≈ 1.2–2.0` — term-frequency saturation
- `b ≈ 0.75` — length normalisation strength

Two properties matter and are worth being explicit about, because they are the backbone of
spam resistance:

- **Saturation.** Each additional occurrence of `t` adds less than the last. Ten copies of
  a keyword are worth far less than ten, which is why keyword stuffing fails against BM25.
- **Length normalisation.** A long document does not score higher merely for containing
  more words; the denominator grows with it.

## Why BM25F rather than per-field BM25

A search engine has multiple fields (title, headings, body, anchors, url) and each should
contribute differently. BM25F combines them into **one term frequency per document** before
applying saturation:

```text
TF̃(t, d) = Σ_f  w_f · tf(t, d, f) / (1 − b_f + b_f · len_f(d) / avglen_f)

score(d) = Σ_t  IDF(t) · TF̃(t, d) · (k₁ + 1) / (k₁ + TF̃(t, d))
```

The critical difference from "sum the per-field BM25 scores": each field's raw term
frequency is **normalised by its own length before weighting**, then saturation is applied
once to the combined value. Summing separate BM25 scores instead lets the longest field
dominate, because it accumulates the most raw frequency.

Worked intuition: a title with the term once, in a short field, gets a large normalised
weight from `w_title`; a body with the term forty times gets a large raw weight but is
divided by a long `len_body` and then saturated hard. Title wins — which is the correct
outcome, because a title is an editorial claim and body text is not.

## Field weights

From [../indexing/index-schema.md](../indexing/index-schema.md):

```text
w_title      12.0
w_headings    6.0
w_anchors     4.0
w_url         3.0
w_body        2.0
w_alt         1.5
```

`b_f` (length normalisation) is `1.0` for title and headings — a long title genuinely is
less focused — and `0.5` for body, where full normalisation would unfairly penalise long
articles.

## IDF

`df` comes from the index's own term statistics, per field. A term appearing in every
document has IDF ≈ 0 and contributes nothing, which is the desired behaviour for
boilerplate vocabulary.

Two refinements worth having:

- **Field-scoped IDF** for the URL and anchors fields, since their vocabularies differ
  radically from body text.
- **IDF floor** so a term that happens to be rare in a small corpus is not given an
  unreasonable advantage. Without a floor, a single document containing an unusual token can
  dominate a whole query.

## Phrase and proximity

```text
"exact phrase"     → ordered phrase match, slop 0
"loose phrase"     → unordered, all terms within a sliding window (default 8 tokens)
```

Because positions are stored per field, exact phrase matching is a position-join rather
than a heuristic. Slop is a graded fallback:

```text
phrase score = bm25f_sum × (1 + φ) × position_bonus
position_bonus = 1.0 exact · 0.6 within window · 0.3 all terms present, order free
```

A document containing the exact phrase is preferred to one containing the same words
scattered, but a document with all the terms is still relevant.

## Query-term handling

| Situation | Behaviour |
| --------- | --------- |
| Term appears in `df < 3` documents | Keep, with the IDF floor applied |
| Term not in the index at all | Drop it; if no terms remain, return an empty result set with a `NO_MATCHES` signal |
| Term after spell correction | Original term is retained alongside as a `should`, so a correction cannot destroy a result |
| Term expanded (synonym/inflection) | Expanded terms weight `0.4` relative to the original |
| Stopword-only query | Valid; scored with stopword weights, not treated as empty |
| Repeated query terms | `qf` rises, which correctly rewards documents containing both |

## Tuning

```text
k₁  1.4    saturation point; raise to blunt stuffing further
b   0.75   global default; 1.0 for short fields (title/headings), 0.5 for body
k₁' (fused) 2.0    saturation on the fused TF
```

Tuning is empirical, on the judged set, with a golden diff review. The intuition to hold
onto:

- Raising `k₁` weakens saturation — more stuffing pays off. Lower `k₁` strengthens it.
- Raising `b` strengthens length normalisation — long documents are penalised more.
- Raising `w_body` shifts weight from editorial signals to bulk text, which correlates with
  lower spam resistance.

## Known weaknesses

Stated plainly, because the fix is a later signal rather than a pretence:

| Weakness | Handled by |
| -------- | ---------- |
| Term-frequency stuffing | Saturation, plus [spam.md](./spam.md) penalties |
| Exact-match-only semantics | Fuzzy candidates (edit distance ≤ 1–2, dictionary-gated) on the `should` clause |
| Synonym blindness | Curated expansion, then semantic retrieval evaluated offline |
| No proximity preference for unordered matches | Slop fallback with a graded bonus |
| Field stuffing (title repetition) | Per-field length normalisation; `title` length sanity checks at parse |

## Testing

- Unit tests for the scoring function against hand-computed values, including the boundary
  cases (`tf = 0`, `dl = 0`, `dl = avgdl`, single-document corpus).
- Property tests: score is non-decreasing in `tf`; non-increasing in `dl` (at fixed `tf`);
  a document matching more query terms scores at least as high as one matching fewer (holding
  `dl` fixed).
- A regression test asserting that a keyword-stuffed document scores below an equivalent
  natural document — the property that matters.
- Golden snapshots for a fixed corpus, with a CI gate on NDCG@10 and MRR@10.