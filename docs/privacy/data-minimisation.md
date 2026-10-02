# Data minimisation

The general principle behind every specific decision elsewhere: **collect less, keep less, and
be able to say precisely what exists.** This document is the reasoning layer, so a future
contributor can check a proposed feature against the principles rather than against a list.

```text
question a feature must answer before it is designed:
  what data does this need?
  why does it not need less?
  how long does it need it?
  what happens when it is deleted?
  what breaks if we never had it?
```

That last question is the useful one. A feature that cannot state what it would still do
without the new data is a feature that is choosing collection over design.

## Principles

| # | Principle | Consequence in practice |
| - | --------- | ----------------------- |
| 1 | Do not collect what was not asked for | Snippets come from the document, not from a query log |
| 2 | Prefer derived to stored | Page count is a counter, not a list of queries |
| 3 | Bound retention by usefulness, not convenience | 300 s for cache, because that is what latency needs |
| 4 | Delete by default, restore on request | Tombstones first; verification second |
| 5 | Aggregation is not privacy | A table of "top searches by IP" is a query log with extra steps |
| 6 | Pseudonymisation is not anonymisation | IP hashing reduces exposure; it does not make the data safe to keep |
| 7 | Metadata is data | User agent, referrer, and IP are all behavioural signals |
| 8 | Local-first is a real reduction | Anything that can be computed client-side is not sent |
| 9 | Unstore means never wrote it | Deletion is weaker than never collecting |
| 10 | Minimize the operator too | An operator with production access to user behaviour is a liability |

The tenth is unusual and worth defending: privacy commitments usually protect users from third
parties. LYNX also limits what its own operators can see, because the operator is the third
party most likely to be careless.

## Minimisation by subsystem

### Request path

| Collected | Not collected |
| --------- | -------------- |
| The query, in memory, for the duration of the request | The query in any store |
| IP for rate-limiting, bucketed with a daily-rotating salt | Raw IP retention |
| `Accept-Language` for interface language | `User-Agent` beyond a coarse client class |
| Nothing else | Referrer, cookies, client fingerprint, account id |

The coarse client class matters: the raw user agent string is a fingerprinting vector, and
serving it back to a CDN or a library would leak. LYNX maps it to `desktop | mobile | bot |
unknown` at the edge and discards the rest.

### Crawl path

The crawler handles public web pages on behalf of nobody. Minimisation applies to what it
retains:

| Kept | Discarded |
| ---- | --------- |
| Response headers, for protocol decisions | Headers irrelevant to fetching and caching |
| Parsed content | Raw HTML, raw body bytes |
| Extracted links and their attributes | Inline scripts, styles, images |
| robots.txt, per spec | Nothing — robots.txt is required for compliance |
| Content hash, for change detection | No full page archive |

Images are not stored at all. LYNX does not build an image index, so fetching and retaining
them would be pure cost and pure liability.

### Index path

| Kept | Discarded |
| ---- | --------- |
| Extracted text with positions | The DOM |
| Structured metadata, whitelisted by type | Arbitrary JSON-LD that matches no known type |
| Field and fast-field statistics | Per-document raw attribute maps beyond the declared schema |

### AI path

The optional AI layer is the sharpest edge in the minimisation model, because retrieval means
reading query-adjacent content.

| Rule | Detail |
| ---- | ------ |
| No query text to an external model | Astra is self-hosted; there is no external API call at all |
| Retrieval is over the public corpus | Not over anything private, because nothing private exists |
| No prompt or completion logging | Prompts contain query terms |
| Model output is discarded after rendering | No conversation history |
| No fine-tuning on user queries | There is no user-query data |

## Deliberate omissions

Some things a search engine could store that LYNX does not:

| Omitted | What it would enable | Why omitted |
| ------- | ------------------- | ----------- |
| Query frequency | Spelling correction from real misspellings, trending topics | Log-free autocomplete is worse but honest |
| Click-through data | Learning-to-rank | This is the industry-standard method and it requires exactly what we refuse |
| Personalised history | Better results for the individual | No profile store exists |
| Session reconstruction | Funnel analysis | Not useful for a search engine except as surveillance |
| Exact geolocation | Local results | City-level intent from the query is enough |
| Full user agent | Device-specific rendering | Fingerprinting surface |
| Page archive | Cached-page view, historical analysis | Copyright and liability |
| Full HTTP archive (WARC) | Reproving what a page said | Same, at scale |

The click-through omission is the expensive one. Learning-to-rank is the largest quality lever
in search, and every implementation of it trains on interaction data. LYNX's ranking quality
ceiling is a direct, accepted consequence of that refusal.

## Third-party data

LYNX collects **no** third-party data. No purchased feeds, no licensed corpora, no shared
indexes, no third-party crawl data, no reputation feeds, no IP geolocation beyond coarse
derivation, no prefetched link graph.

The reason is a compound one: a third-party data agreement is an ongoing legal relationship
that must be renewed, honoured, and eventually ended, and ending it does not undo the data
already absorbed into an index. Independence is easier to maintain without obligations.

This has costs: coverage is limited to what LYNX crawls, and there is no bulk seed corpus. It
also has a benefit that is easy to state and easy to verify — LYNX's index is entirely
self-collected, so its provenance is fully known and its removal obligations are fully
contained.

## Future data questions

For any proposed new data store:

1. Can it be computed on demand instead of stored?
2. Can it be computed locally instead of transmitted?
3. Can it be aggregated to something coarser?
4. Does it survive a retention bound short enough to matter?
5. What is the deletion procedure, and can it be verified?
6. Does it require an ADR in `docs/adr/`?
7. Would a user who knew it existed object?

Question 7 is not a joke. If a plausible user would object to its existence, the answer is
usually no, and the deliberation is faster than the objection.

## Review

| Review | Frequency |
| ------ | --------- |
| This document against the code | Quarterly |
| Every new store against principle 1 | Every design review |
| ADR check for any new persistent data | Every design review |

Last reviewed: see [git history for this file](../../).