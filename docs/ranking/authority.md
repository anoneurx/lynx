# Authority

Authority estimates how likely a page is to be useful and trustworthy, derived from the link
graph. It is the least trustworthy signal in LYNX and the one most often gamed by SEO, so it
is weighted low, capped, and always combined with quality and spam signals.

## The link graph

```text
DOCUMENT ──outlink──▶ URL ──▶ DOCUMENT
    ▲                                    │
    └────────────────────────────────────┘
```

Edges live in the `link` table with: source `doc_id`, target `url_id`, anchor text, first
seen, last seen, and `is_nofollow`. No JavaScript execution, so no client-side-rendered links
— a real coverage and accuracy limitation, stated rather than hidden.

## PageRank

```text
PR(d) = (1 − D)/N  +  D · Σ_{i → d} PR(i) · outweight(i)

D          = 0.85 damping
outweight(i) = 1 / outdegree(i)     for <dofollow> links
outweight(i) = 0                   for <nofollow> links and to excluded hosts
N          = number of documents in the computation graph
```

Properties that make it worth using:

- **Random teleport** — the `(1−D)/N` term means a page with no inbound links still has
  non-zero rank, and the graph cannot be fully captured by a closed link ring.
- **Value flows through links**, so a link from a high-rank page is worth more. This is the
  behaviour that makes it hard to manufacture authority from nothing.
- **Damping bounds influence**: no page can exceed `(1−D)+D = 1`, so a ring cannot push rank
  beyond the ceiling.

## Implementation

Computed **offline in batches**, not at query time:

```text
1. build an edge list from the link table (deduplicated, nofollow excluded)
2. exclude blocked, banned, and spam-flagged documents
3. 30–50 iterations of damped PageRank over the edge list
4. normalise to [0, 1] via log compression: pr_norm = ln(1 + pr·N) / ln(1 + max_pr)
5. aggregate to host_rank and domain_rank
6. write as fast fields into a new index generation
```

Log compression is what makes PageRank usable as a bounded ranking feature. Raw PageRank is
extremely long-tailed — the median document's rank is orders of magnitude below the top
page's, so raw values are unusable without it. `ln(1 + pr·N) / ln(1 + max)` maps the whole
distribution into [0, 1] while preserving ordering.

## Domain authority

```text
domain_rank = log-mean of the top-K (K=50) document ranks in the registrable domain
              capped at domain_cap (default 0.6)
```

Aggregating to the domain and capping is the single most important anti-gaming measure:

- **Log-mean, not max.** A domain with one excellent page and ten thousand spam pages gets a
  modest score, not a great one. Max aggregation would hand authority to any domain that
  publishes one good page.
- **Top-K only.** The tail of a domain's documents does not drag the score, but it also
  cannot inflate it.
- **A hard cap.** Even a genuinely authoritative domain cannot push `authority` above 0.6,
  because authority is 0.15-weighted and must never dominate lexical relevance. A site
  cannot buy its way to the top of every query.

Registered domains use a public-suffix list, so `example.co.uk` and `example.com` are
distinct, and a subdomain cannot inherit its parent's authority (a cheap spam vector).

## Backlinks and link quality

```text
unique referring domains      count, log-scaled
external outlink ratio        outlinks to other registrable domains / total outlinks
reciprocity rate              fraction of outlinks that are also inlinks
anchor-text entropy           diversity of anchor text (uniform "click here" = suspicious)
nofollow ratio                administrative, not a quality signal by itself
link age                      a link from a page crawled 8 years ago is weaker evidence
```

Reciprocal-link rings are the standard attack, and the ratio detects them: two sites linking
to each other with a high proportion of their outlinks is not independent endorsement.

## What is explicitly excluded

| Excluded | Why |
| -------- | --- |
| `javascript:` links | never followed, never in the graph |
| `nofollow` links | the author asked us not to treat them as endorsement |
| Links from banned or spam-flagged documents | otherwise spam becomes a distribution channel for authority |
| Links from parked domains, and from known link-farm templates | identified at cluster level |
| Author-supplied `PR` meta tags or similar | site-asserted authority is not authority |
| Subdomain → parent inheritance | spam vector |
| Paid/sponsored link markup | detected via `rel=sponsored`; excluded from the graph |

## Freshness interaction

Authority does not imply freshness and does not decay on a fixed schedule. It decays with the
link graph's recency: links seen in the current crawl generation count fully, links not
re-observed in several generations are weighted down, and links not re-observed in a year
drop out. Authority is therefore a statement about the *current* web, not a permanent
attribute.

## Interaction with spam

Authority is the signal most likely to be wrong, so it is designed to fail safe:

```text
spam_score high  →  spam_penalty multiplies the whole score, including authority's
                  →  a link-farm page with PageRank 0.8 still ranks low
```

`w_authority = 0.15` is deliberately low. In a mature index, BM25F should be doing most of
the work; authority should be a tie-breaker between comparably relevant pages, not a
substitute for relevance.

## Local and personalised PageRank

Not implemented. Topic-biased PageRank would improve precision for some queries, and it
would also require deciding LYNX's topic taxonomy — a judgement about the whole web that
belongs to a product decision, not a quiet engineering choice. If it is ever added, it is an
ADR plus a measurable hypothesis in `research/`.

## Known weaknesses

| Weakness | Assessment |
| -------- | ---------- |
| Link farms still get authority | Mitigated by capping, reciprocity detection, cluster similarity, and the multiplicative spam penalty. Rings that evade all four are accepted |
| JavaScript-rendered links are invisible | Real coverage loss. Accepted; no browser execution |
| `nofollow` is inconsistently applied | Some sites `nofollow` everything, which under-rates them |
| A newly popular site needs time to gain authority | Accepted latency; freshness and lexical relevance carry it early |
| Authority is global, not topical | A domain authoritative about one subject looks authoritative about all. Partially mitigated by BM25F doing the topical work |
| Small-index PageRank is noisy | Early in a corpus's life, authority is close to meaningless. Weighted low enough to be safe, and a quality signal suppresses it for very young indexes |

## Testing

- Unit tests on a hand-computed tiny graph (3–5 nodes with known expected ranks).
- Property tests: rank is invariant to edge order; adding an outlink from a high-rank node
  does not decrease the target's rank; every rank is in (0, 1); the sum of ranks equals N
  after normalisation.
- A convergence test asserting the residual change falls below threshold in the chosen
  iteration count.
- A gaming regression suite: a synthetic link farm, a reciprocal ring, and a domain with one
  good page and many spam pages must all score below the natural equivalent.
- A distribution monitor: a sudden shift in the authority histogram is an alert, because it
  usually means a new link scheme rather than a change in the web.