# Quality

On-page quality signals: how likely is this page to be a real, useful, maintained document
rather than a generated artefact. Cheap to compute, already stored as fast fields, and
useful precisely because they are honest about what they measure.

## Signals

| Signal | Range | Rationale |
| ------ | ----- | --------- |
| `title_present` | 0/1 | A page without a title is usually a template fragment |
| `title_length_ok` | −0.5…1 | 10–70 characters. Too short is not informative; too long is truncated in the SERP |
| `meta_description_present` | 0/1 | Proxy for deliberate metadata |
| `canonical_present` | 0/1 | Proxy for duplicate awareness |
| `https` | 0/1 | Weakly correlated with maintenance; not a trust claim |
| `content_length_ok` | −1…1 | Below ~300 words is thin; the sign reflects thinness |
| `link_density_ok` | −1…1 | Body with 60 % links is a link list, not an article |
| `text_html_ratio` | 0…1 | From parse features |
| `lang_declared` | 0/1 | Declared and detected language agree |
| `lang_confidence` | 0…1 | Detector confidence |
| `published_at_present` | 0/1 | Dated content is usually maintained content |
| `last_modified_gt_published` | 0/1 | Evidence of maintenance |
| `structured_data_valid` | 0/1 | JSON-LD parses and validates |
| `js_required` | −1…0 | A page needing JavaScript to show content will render thin for us |
| `not_truncated` | 0/1 | We hit a parse limit; quality is unknown, so treat it as a caution |
| `not_soft_404` | 0/1 | Content matched the site's own 404 signature |
| `unique_outlinks_ratio` | 0…1 | Links to genuinely distinct destinations |

```text
quality = Σ  w_i · signal_i        all clamped, Σw = 1
```

`w_title_present` and `w_quality` get the largest weights. A page with no title, no
description, 80 words of content, and 200 links is not a page; it is an artefact.

## What quality is not

Explicitly **not** signals, because each is either trivially gamed or measures the wrong
thing:

| Not a quality signal | Why |
| -------------------- | --- |
| PageRank / link count | That is [authority](./authority.md), and conflating them double-counts |
| Alexa-style site rank | Third-party data, licensing, and staleness |
| Domain age | Weakly correlated, easy to game with a new domain, and mostly measures how long someone has been at it |
| WHOIS / registration data | Privacy-invasive, legally fraught, out of scope |
| Whois privacy-protected flag | Says nothing about content |
| Author identity | Unverifiable, and profiling-adjacent |
| Keyword density | A stuffing detection input, not a quality input — see [spam.md](./spam.md) |
| `meta keywords` | Author-asserted and universally ignored; LYNX does not index it |
| HTTPS certificate details | Deployment trivia, not content quality |
| Page speed | Genuinely useful, but requiring measurement infrastructure we do not have; revisit if it becomes measurable |
| Author claims, awards, "expert" markers | Unverifiable self-assertion |

The common thread: quality signals describe **the page as we received it**, not claims the
page makes about itself.

## Normalisation and calibration

Each signal is mapped to [−1, 1] against explicit thresholds that live in config, so the
calibration is reviewable and adjustable without touching the code:

```toml
[ranking.quality]
min_words              = 300
good_words             = 800
max_link_density       = 0.35
title_min_chars        = 10
title_max_chars        = 70
```

Calibration happens against a hand-labelled sample of ~500 documents across quality tiers,
checked in as a fixture. The `quality` distribution over the whole corpus is tracked
monthly; a shift means either the corpus changed or the thresholds need revisiting — and
that monitoring is what tells the two apart.

## Relationship to other signals

```text
quality   →  additive   (a good page deserves a boost)
authority →  additive   (independent axis)
freshness →  additive   (independent axis)
spam      →  MULTIPLICATIVE (a spam page cannot be rescued)
```

That asymmetry is deliberate. Absence of quality signals is weak evidence; presence of spam
signals is strong evidence. Treating them with the same arithmetic would let either one
dominate, and would make the system easier to game in both directions.

## Effect on the SERP

Quality is invisible to the user except through ordering. It is not a badge, a star rating,
or a "high quality" label, because LYNX has no basis for asserting such a label to the user.
It is a ranking input and it is visible in the explain output for operators.

## Known weaknesses

| Weakness | Assessment |
| -------- | ---------- |
| Thin content can be padded to look fine | `link_density_ok`, `text_html_ratio`, and duplicate-cluster detection catch most padding; padded-but-original content is genuinely hard to detect |
| A page can satisfy every signal and still be bad | True, and the honest answer is that on-page quality is a weak signal. It is weighted 0.12 for that reason |
| `js_required` is a blunt negative | A JS shell page with server-rendered content is misjudged. The penalty is small and the flag is also fed to the AI layer's trust scoring |
| Templates produce false negatives | A well-built template scores well on every signal. That is acceptable — templated sites are not necessarily low quality |
| Signals drift as the web drifts | Monthly distribution monitoring plus the fixture-based calibration sample |

## Testing

- A labelled fixture set with a known quality distribution; the calibration is a test, not a
  comment.
- Property tests: quality is in [−1, 1]; adding a title cannot decrease quality; a page at
  the thin-content boundary scores below one above it, monotonically.
- A regression test that a padded thin page scores below a comparable natural page.
- A distribution monitor with an alert on a shift beyond the historical band — the only way
  to notice that "average quality" quietly changed.