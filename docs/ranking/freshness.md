# Freshness

Freshness is a context-sensitive signal, not a global one. A document from 2005 is
authoritative for a historical question and useless for a current one. LYNX treats freshness
as a function of the query and the document type.

## Signals

```text
published_at     from metadata, JSON-LD, <time>, URL patterns
modified_at      content revision, when distinguishable from publication
crawled_at       when LYNX last fetched it
content_hash_age whether the content has actually changed since last crawl
```

Precedence: `modified_at` when present, else `published_at`. When nothing is detectable,
`published_at` is null and freshness falls back to crawl recency — a weaker signal, marked as
weaker in the signal vector.

**A date is never guessed.** Inferring a publication date from a filename or a copyright
footer would poison freshness ranking permanently, because the wrong date becomes a
permanent ranking fact. Null is a valid answer.

## Time-sensitive intent

Some queries want fresh results, most do not. LYNX detects intent from the query, not from
the user:

```text
explicit   before: / after: / freshness=  →  hard filter, no decay applied
strong     "latest", "current", "today", "now", "2026", "this week",
           "still", "update", "release", "version", "price", "status"
           →  freshness weight × 3, no re-ranking of age beyond the decay
weak       "recent", "new", "latest version of"    →  freshness weight × 1.5
none       everything else                            →  freshness weight × 1.0
```

Detection is a term match against a curated list plus the explicit operators. It is
deliberately simple and inspectable: a user asking "latest rust release" should not receive
a 2019 blog post above the current release page, and a user asking "history of the rust
language" should not receive only last month's posts.

## Decay

```text
raw_decay(age_days, half_life_days) = 0.5 ^ (age_days / half_life_days)

freshness = w_published · raw_decay(age_published, hl_published)
          + w_modified  · raw_decay(age_modified,  hl_modified)
          + w_crawled   · raw_decay(age_crawled,   hl_crawled)
```

Exponential decay on a half-life, because a constant-rate decay does not exist in reality
and a linear decay would let a five-year-old document score above a two-year-old one by an
unbounded margin.

Weights are `0.5 / 0.3 / 0.2`; the crawler-recay term has the longest half-life (365 days)
because crawl recency reflects *our* attention, not the page's importance.

## Half-lives by content type

| Content type | `hl_published` | Rationale |
| ------------ | --------------- | --------- |
| news, blog    | 7 days   | a week-old article is stale news |
| release notes, changelog | 30 days | version cycles |
| documentation | 180 days | stable for months, then revised |
| reference    | 365 days | slowly changing |
| Q&A          | 120 days | answers expire as software changes |
| academic     | 730 days | slow-moving, date is publication not validity |
| unknown      | 90 days  | the honest default |

The content-type classification comes from page structure and metadata
(`article:published_time`, JSON-LD `articleSection`, URL patterns), not from a per-domain
hand-list — a hand-list would not scale past the first few thousand domains and would be
tedious to keep honest.

## Time-sensitive handling

For strong freshness intent, decay is applied more aggressively and a minimum freshness
floor is imposed:

```text
strong intent:
  effective_hl     = hl / 3
  floor            = 0.25       # a very old document is heavily discounted
  crawl_recency_boost = max over shards of how recently this domain was refreshed
```

The floor is the part that makes "latest rust release" work. Without it, a highly
authoritative page from 2015 can outrank the actual current release regardless of how fresh
it is.

## Interaction with the rest of ranking

Freshness is one weighted term among five. Deliberate consequences:

- Freshness cannot overpower a strong lexical mismatch. `w_freshness = 0.18` against a
  lexical scale normalised to ~1 means a perfect topical match with stale content beats a
  weakly related fresh page.
- Freshness does not rescue spam. Spam penalties are multiplicative and applied after.
- Authority does not imply freshness. An authoritative domain publishing stale pages stays
  demoted for time-sensitive queries; that is correct.

## Freshness reporting

`/api/v1/status` exposes index freshness aggregates:

```json
{ "index": { "generation": "…", "median_crawl_age_days": 3.2,
             "p90_crawl_age_days": 41.0, "documents_crawled_last_24h": 184203 } }
```

And a monthly operations report records median and p90 age by content type, because a
freshness regression that shows up only in a percentile is invisible in a daily count.

## Known weaknesses

| Weakness | Assessment |
| -------- | ---------- |
| Publisher-supplied dates can be wrong | Unfixable from our side. Mitigated by preferring `modified_at` and by not trusting date-only signals from a domain with a history of inaccuracy |
| No decay for content type change | A docs page that becomes news keeps the docs half-life until reclassified |
| Freshness intent detection is lexical | "current" in a query about a current affairs book is a false positive; cost is one weight multiplier, not a hard filter |
| Crawl recency reflects our coverage, not the page's quality | A well-crawled spam page looks fresher than a neglected legitimate page. This is why `w_crawled` is the smallest weight |

## Testing

- Unit tests for the decay function at the half-life boundary, at zero age, and at negative
  age (clock skew must clamp, not produce a value > 1).
- A regression test asserting a current release note outranks a 2015 blog post for
  "latest rust release", and that the 2015 post outranks nothing for
  "history of the rust language".
- A golden test for date extraction from a corpus of date-bearing fixtures.
- A metric gate on median age of top-10 for the time-sensitive query subset.