# Spam detection

Spam is the main reason a search engine gets worse as it grows. LYNX treats spam detection as
a ranking input with named, inspectable signals — not an opaque classifier that silently
decides what users see.

## Why multiplicative

```text
score_final = score_raw × (1 − clamp(spam, 0, 1) · w_penalty)
```

An **additive** spam penalty can be overcome by a strong lexical match. A page engineered to
match a query exactly would keep its score and stay in the results. Multiplicative
application means spam scales the whole score down: a page that is both highly relevant by
lexical score and heavily spam-flagged ends up below a merely relevant clean page. That is
the correct behaviour, and it is why `w_penalty` is the one weight with the most leverage.

The cost is real and worth naming: a false positive is severely punished. Therefore the
penalty is bounded (`w_penalty = 0.40`, so a fully-flagged page keeps 60 % of its score
rather than being zeroed), and the penalty gate is a quality-metric gate in CI.

## Signals

### Content signals

| Signal | Detection | Range |
| ------ | --------- | ----- |
| `keyword_stuffing` | term entropy, title-to-body term ratio, repeated n-gram rate | 0…1 |
| `low_text_diversity` | type-token ratio below threshold for the length | 0…1 |
| `doorway_cluster` | template signature match across > 50 documents in a domain | 0…1 |
| `thin_content` | word count below threshold with a marketing intent shape | 0…1 |
| `duplicated_block` | internal repetition of sentence-level shingles | 0…1 |
| `hidden_text` | text present in the DOM but not visible (detected at parse) | 0/1 |
| `unnatural_headings` | heading structure repeated across pages of a domain | 0…1 |

### Link signals

| Signal | Detection | Range |
| ------ | --------- | ----- |
| `link_farm_participant` | reciprocal-link ratio, in/out degree anomaly, template link signature | 0…1 |
| `low_external_ratio` | almost all outlinks internal | 0…1 |
| `anchor_stuffing` | uniform commercial anchor text across outlinks | 0…1 |
| `sudden_link_spike` | authority growth inconsistent with crawl history | 0…1 |
| `irrelevant_inbound` | inbound links from unrelated template clusters | 0…1 |

### Behaviour signals

| Signal | Detection | Range |
| ------ | --------- | ----- |
| `cloaking` | content signature variance across crawl occasions and contexts | 0…1 |
| `fast_content_churn` | large share of the domain's URLs are new on every crawl | 0…1 |
| `no_maintenance` | crawl success rate collapses while the domain remains linked | 0…1 |

## The three hardest problems, and honest positions on them

### Keyword stuffing

```text
stuffing = f(term entropy, title_tf/body_tf ratio, repeated 5-gram rate,
             tf distribution Gini coefficient)
```

BM25's saturation already handles most stuffing — the tenth occurrence adds a fraction of
the second. The stuffing signal catches what saturation misses: stuffing that *looks* natural
to a saturating scorer by being spread across many low-frequency related terms, which is the
current state of the art in SEO content.

### Doorway pages and template clusters

The signature approach in [../indexing/dedup.md](../indexing/dedup.md) is the strongest
tool available: structural shingles over the DOM, excluding text, so that thousands of pages
that differ only in a substituted word collapse to one signature.

The genuine difficulty is **distinguishing a doorway network from legitimate templated
publishing** — a press release syndicated identically to fifty sites has the same signature.
LYNX combines the signature with link behaviour, domain age, and content volume rather than
penalising the signature alone. This will produce both false positives and false negatives,
and it is the least reliable part of the system.

### Cloaking

```text
variance = dispersion of content signatures for one URL across crawl occasions
          combined with divergence between the crawler user agent and a neutral one
```

The honest limitation: a site can detect that it is being served a crawler's user agent or
an unusual network and serve clean content. Repeated crawls from varied contexts are the
best available signal, and they are defeatable by any site willing to work at it. Detecting
it reliably would require rotating identities, which would compromise our honesty about
identity and our politeness. **LYNX accepts the gap rather than solving it dishonestly.**

## Penalties

```text
spam_score = clamp( Σ  w_i · signal_i , 0, 1 )     weights in config

ranking_penalty = 1 − spam_score × w_penalty
```

```toml
[ranking.signals]
spam_penalty = 0.40

[ranking.spam_weights]
keyword_stuffing      = 0.20
doorway_cluster       = 0.25
link_farm_participant = 0.20
cloaking              = 0.15
hidden_text           = 0.15
thin_content          = 0.05
```

Any single signal can be suppressed by the spam score of a domain, but not by itself: signals
combine additively into a clamped total, so several weak indicators can reach the same
conclusion without any one of them being able to dominate.

Beyond the score, three discrete suppressions apply to the index rather than the ranking:

| Suppression | Effect |
| ----------- | ------ |
| `noindex` | never indexed, per site instruction |
| Banned domain | never fetched, never indexed, links not counted for authority |
| Soft-404 | indexed as a 404, not a result |

## Reporting

Spam is not user-visible as a label. What a user gets is:

- Spam-flagged pages ranked low, and usually absent.
- A **result-level explain** for operators showing the spam score and which signals fired.
- A **domain-level report** in the admin console: spam score, signals, first seen, crawl
  volume.
- A **user-reported spam** path that feeds operator review (never automatic
  suppression — user reports go to a human, because automated takedown is an abuse vector
  in itself).

## Operating the classifier

```text
weekly:   spam score distribution by domain; pages moving in/out of the top 10
          flagged domains awaiting review
monthly:  precision estimate on a human-labelled sample (target ≥ 0.9)
          false-negative hunt: sample of unflagged top-10 results
per PR:    gate on spam-flagged rate in top-10 for the judged set — any increase blocks
```

The false-negative hunt matters more than the precision number. Missing spam is what users
experience, while a false positive is what operators notice and can fix. Both are reported;
the hunt is deliberate.

## Anti-patterns we avoid

| Anti-pattern | Why it is wrong here |
| ------------- | --------------------- |
| An unexplainable ML classifier | Violates the project's central ranking commitment |
| A site-asserted reputation database | Author-asserted quality is not quality |
| Automatic permanent suppression on a heuristic | Punishes sites for our uncertainty |
| Punishing a whole domain for one bad page | A domain can host one spam page and thousands of good ones |
| Using spam detection to hide quality gaps | Better to be worse than to be quietly manipulative |
| Training on user search behaviour | Forbidden by the privacy model |

## Testing

- A labelled spam/non-spam fixture set with a precision target, checked into the corpus.
- A **synthetic gaming suite**: for each gaming technique, a natural equivalent that must
  score above the gamed version. The natural/gamed pair is the test — asserting only that
  spam is punished proves nothing about false positives.
- Property tests: `spam_score ∈ [0, 1]`; penalising can only decrease a score;
  `spam = 0` leaves the score unchanged.
- A metric gate on the spam-flagged rate in the top 10 of the judged set.
- A test that spam-flagged documents contribute nothing to authority, closing the loop with
  [authority.md](./authority.md).