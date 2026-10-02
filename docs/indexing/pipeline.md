# Indexing pipeline

Every stage, in order, with its failure behaviour. A stage that silently drops data is a
bug; a stage that records why it degraded is engineering.

## Stage 0 — ingest

```text
input:  ParsedDocument (from services/parser)
output: queued for analysis
checks: parser_version recorded; document_version row created
```

The version row is created before the document is written, so a crash leaves a recoverable
state rather than a document with no provenance.

## Stage 1 — normalisation

```text
1. strip C0/C1 control characters (keep \t and \n)
2. strip Unicode format characters: zero-width, bidi overrides/isolates, soft hyphen
3. normalise to NFC
4. collapse whitespace runs to single spaces
5. fold full-width forms
6. case-fold for indexing (locale-independent)
```

Step 2 is not cosmetic. Zero-width characters are a classic index-poisoning and
click-through-redirection technique, and bidi overrides can make stored text render
differently from what it means. Removing them at index time means the stored and served
text agree.

Homoglyph folding is **not** applied to indexed text — see
[parser.md § sanitisation](../crawler/parser.md).

## Stage 2 — language confirmation

```text
parser language estimate  ┐
html lang attribute       ├─→ agreement check → language + confidence
Content-Language header   │
hreflang alternates      ─┘
```

Disagreement is not an error. It sets `language_confidence` low and applies a quality
penalty. A page that lies about its language while its text is clearly Chinese is a signal
worth having.

Multi-language pages are segmented by language run and stored in per-language sub-fields, so
a Japanese query does not match a page whose English navigation bar contains the term.

## Stage 3 — tokenisation

Per-field tokenisers. See [tokenization.md](./tokenization.md) for the algorithms.

| Field | Tokeniser | Notes |
| ----- | --------- | ----- |
| title | word + CJK bigram | Highest weight |
| headings | word + CJK bigram | |
| body | word + CJK bigram | Saturated |
| anchors | word | |
| url | path-tokenised | Schemes and separators dropped |
| alt / caption | word | Low weight |
| noindex metadata | not indexed | |

CJK has no spaces, so word segmentation does not apply; LYNX uses character bigrams, which
is the standard pragmatic approach without requiring a dictionary-based segmenter.

## Stage 4 — stopwords

Stopword lists are **language-specific and index-time**. Stopped tokens are retained in a
separate low-weight field rather than deleted, so exact phrase queries still match. Deleting
them would break `"the who"`-shaped queries and would make phrase scoring wrong.

## Stage 5 — stemming

```text
English   Porter2
German    Snowball (German)
French    Snowball (French)
Spanish   Snowball (Spanish)
Italian   Snowball (Italian)
Russian   Snowball (Russian)
Portuguese Snowball (Portuguese)
Nordic    Snowball (Swedish, Danish, Norwegian)
Turkish   a light suffix-stripping rule (agglutinative; naive Stemmer over-stems)
CJK       none (bigrams already)
other     none
```

Index-time only. The query pipeline applies the same stemmer to query terms with the same
version. Both sides read the version from the index manifest, so a mismatch is impossible
in practice and detectable in tests.

## Stage 6 — term statistics

```text
tf per field              raw term frequency in that field
positions                 token positions per field (bounded to 512 positions/field)
field length              tokens per field, for BM25F normalisation
doc length (body)         for global length normalisation
doc length (all indexed)  aggregate for weight calibration
```

Position caps matter: a 200 000-word page would otherwise store 200 000 positions per term.
Positions beyond the cap are dropped, and a flag marks the document as position-truncated
(phrase queries degrade to unordered matching for that document).

## Stage 7 — dedup

```text
simhash (64-bit) → banded LSH lookup (4 bands of 16 bits) → candidate set
candidate set    → MinHash (64 permutations) Jaccard estimate
                 → if ≥ threshold (0.85), join the candidate's cluster
                 → else create a new cluster
```

Details and thresholds: [dedup.md](./dedup.md).

## Stage 8 — field assembly

Each field's tokens are written with the field boost from
[index-schema.md](./index-schema.md). A document is written once with all fields; there is
no per-field document.

## Stage 9 — write and commit

```text
tantivy doc:
  id            = doc_id (uuid v7 from canonical url)
  per-field     = analysed tokens + positions + field norms
  stored        = title, url, display_url, snippet source, language, dates,
                  quality features, cluster_id, domain_id, host_id
  ranking_meta  = authority (host_rank, domain_rank), spam_score, quality_score,
                  freshness bucket, ranking_config_version
  index-time    = published_at, modified_at, crawled_at
```

Commit policy: every 20 000 documents or 30 seconds, whichever comes first, plus an
explicit flush before a snapshot. Commits are the expensive part of indexing, and a
consistent commit cadence is what keeps the commit latency histogram tight.

## Failure behaviour

| Failure | Behaviour |
| ------- | --------- |
| Tokenisation impossible (empty text) | `index_rejected: empty_text`, document row kept, no index entry |
| Language undetectable | index under `und` with confidence 0, quality penalty applied |
| Field length cap hit | truncate, set `truncated` flag, keep indexing |
| Position cap hit | drop excess positions, set `position_truncated`, keep indexing |
| Simhash collision storm (band > 4 000 documents) | fall back to MinHash-only over the band's ids; flag the doc |
| Index write failure | retry with backoff (our bug, not the site's); after 5 failures, park the document with an alert |
| Post-commit failure | the version row is the source of truth; a reconcile job re-derives index state from it |

The last row is the important architectural point: because `document_version` is the
system of record and the index is derived, index corruption is recoverable by re-indexing
without re-crawling. That is what makes the index replaceable in practice rather than in
principle.

## Idempotency

Re-indexing identical content is a no-op: the content hash is compared against the current
version, and an unchanged hash skips the write entirely. This matters for full re-indexes,
which should be able to run without churning the index or triggering unnecessary merges.

## Re-indexing

Three triggers:

| Trigger | Cost | Process |
| ------- | ---- | ------- |
| Parser improvement | Re-fetch (no stored HTML) | Bump `parser_version`, re-crawl by priority |
| Tokenizer change | Full rebuild from document store | New index generation, atomic switch |
| Schema change | Full rebuild | New generation; previous kept 24 h for rollback |

Because the index is derived and generation-switched, all three are the same operation from
an operator's point of view: build a new generation, verify, switch. See
[operations/runbooks.md](../operations/runbooks.md).