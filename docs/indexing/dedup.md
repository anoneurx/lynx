# Duplicate detection

Near-duplicate content is the main reason a search index feels worse than it is. The web has
tens of copies of the same article, parameter-variant URL spam, and templated site clusters.
LYNX detects them and suppresses them rather than deleting them.

## The problem in three shapes

| Shape | Example | Consequence if unhandled |
| ----- | ------- | ------------------------ |
| Exact duplicate | Same article on two sites | 10 results, 1 piece of information |
| Near duplicate | Reworded, reordered, or boilerplate-changed copy | Same, plus snippet noise |
| Template cluster | Thousands of pages from one generator, differing only in a city or product name | Whole-page domination of a query |

The third is the dangerous one, because it is also the signature of doorway pages and SEO
spam. Duplicate handling and spam detection overlap here, and that overlap is deliberate.

## Two-stage detection

### Stage 1 — SimHash for candidate lookup

```text
1. shingle the extracted main text into 5-gram word shingles
2. hash each shingle to 64 bits
3. weight each bit by shingle frequency
4. sum with sign weighting → 64-bit SimHash
```

SimHash has the property that near-duplicates differ in few bits. Candidate lookup uses
**banded LSH**: split the 64 bits into 4 bands of 16 bits, index each band, and union the
posting lists. Two documents sharing any band are candidates.

Banding trades recall for speed: a candidate must collide in at least one band. With 4 bands
of 16 bits, documents within ~15 bits of each other collide with high probability. That is
the right band count for a threshold around 0.85 Jaccard, and it is tuned against a
measured corpus rather than guessed.

**Collision storms are expected** in a band of near-identical pages. A band with more than
4 000 ids is a signal that a template cluster exists; the code degrades to MinHash-only
comparison within that band and flags the document rather than blocking the write.

### Stage 2 — MinHash for verification

```text
1. hash shingles with 64 independent permutations
2. keep the minimum hash per permutation → 64-byte signature
3. Jaccard estimate = fraction of matching signature bytes
4. join if estimate ≥ 0.85
```

MinHash estimates Jaccard similarity with a standard error of about 1/√64 ≈ 0.125 at
k = 64. That is too coarse for a 0.85 threshold on its own, which is exactly why it is used
as a *filter after* SimHash banding has already narrowed the field: within a candidate set
of a few dozen, a 0.125 standard error is fine, and any borderline case is broken by a
second pass that compares the actual shingles.

Two-stage cost: stage 1 is O(1) lookups per document, stage 2 is O(64) comparisons against a
bounded candidate set. Stage 1 does the scaling; stage 2 does the deciding.

## Cluster assignment

```text
new document
 → no simhash band collision                        → new cluster
 → band collision, minhash < 0.85                   → new cluster
 → band collision, minhash ≥ 0.85                   → join the candidate's cluster
 → cluster representative is the highest-quality member, not the first crawled
```

Cluster representative selection matters. The representative should be the version a user
would want: the earliest-crawled or highest-authority copy, not the first one the crawler
happened to reach. Quality and authority decide; arrival order does not.

## What LYNX does with a cluster

**Suppression, not deletion.** Every cluster member stays in the index and remains findable.

```text
representative            →  full score, shown normally
other members, top 10     →  score × duplicate_penalty (default 0.25)
other members, beyond     →  eligible to show if the cluster's members are all relevant,
                             ranked below any representative
```

Rationale: deleting duplicates means a user searching for text that exists on ten mirrors
gets one result and no choice. Demoting them means the first result is the canonical page
and the mirrors are available without being prominent. This is also the honest behaviour
when we cannot tell which copy is authoritative.

`duplicate_penalty` is a ranking weight, versioned like every other weight, and the cluster
membership is stored so the decision is auditable.

## Parameter-variant URLs

Distinct problem, same outcome. `?id=1` and `?id=2` producing identical content is a
template cluster by URL structure rather than by text.

```text
canonicalise(query) → sort + dedupe params + strip tracking params
if content hash of ?id=2 equals content hash of ?id=1
   → same cluster, and url_variation_count++ on the cluster
```

The cluster record keeps the parameter name, not the values, so the pattern ("all IDs under
`/product?id=`") is visible to operators without storing an unbounded URL set.

## Template-cluster detection

Broader than exact duplication: pages from one generator that are structurally identical
with a few slots substituted.

```text
structural shingle = DOM path of the block + its tag sequence
                    (text content excluded)
overlap ≥ 0.90 with > 50 documents in the same registrable domain
  → flag template_cluster
  → each document gets a template_signature hash
  → cross-domain match on the same signature → sitecluster signal
```

Cross-domain template signatures are one of the strongest spam signals available, and they
are also legitimate: a syndicated press release appears identically on many sites. LYNX
distinguishes the two by combining template overlap with other signals (link behaviour,
domain age, content volume) rather than penalising the signature alone. See
[ranking/spam.md](../ranking/spam.md).

## Limits and cost

| Control | Value | Why |
| ------- | ----- | --- |
| Shingle size | 5 words | standard; 3 is noise-sensitive, 7 is miss-sensitive |
| MinHash permutations | 64 | balance between cost and the 0.125 standard error |
| Bands | 4 × 16 bits | tuned for the 0.85 threshold |
| Candidate cap per band | 50 000 | a band wider than this degrades to per-band scanning |
| Signature storage | 64 bytes per document | 320 bytes per million documents — acceptable |
| Shingles retained | 256 fingerprint bits | enough to break borderline cases; not the full text |

The dedup sketch is stored alongside the document. Full shingle sets are not retained, so a
borderline case can be re-decided from the stored sketch but not from the original text —
which would have required storing the text a second time. This is a deliberate trade:
occasional mis-clustering, bounded storage.

## Operational effects

| Metric | Meaning |
| ------ | ------- |
| `lynx_index_dedup_cluster_ratio` | share of documents in a cluster; a rising ratio means the corpus is more repetitive |
| `lynx_index_dedup_candidates_per_doc` | average stage-1 candidates; a spike means a template farm |
| `lynx_index_dedup_band_collisions_total{band}` | collision distribution; one band dominating means a cluster problem |
| `lynx_index_template_cluster_total` | template clusters detected |

A `band_collisions_total` spike on a single band is an operational signal: it usually means
one site's generator produced a large set of near-identical pages, which is worth an
operator's attention as both a quality and a politeness matter.

## Testing

- Golden corpus with known duplicate relationships: exact, near (≥ 0.85), reordered,
  boilerplate-only-different, and deliberately-not-duplicate pairs that must *not* merge.
- Property test: adding 5 % of random tokens to a document keeps it in the same cluster;
  replacing 30 % of its content does not.
- Regression guard on `duplicate_rate_in_top_10`, with a CI gate: a rising rate blocks the
  merge that caused it.
- The "must not merge" cases matter more than the "must merge" ones, because a false merge
  hides a legitimate page entirely.