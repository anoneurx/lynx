# Legal Notes

Third-party content and personal data that enters the corpus, and how the system
handles takedown and erasure requests. The privacy posture in general is
[PRIVACY.md](../../PRIVACY.md) and the [privacy model](./privacy-model.md); this
page covers the obligations that survive even when no person has ever used the
engine.

## What third-party content the corpus contains

| Content class | Legal character | Where it lives | Handling |
| ------------- | --------------- | -------------- | -------- |
| Extracted page text | Third-party copyright, incidental personal data | The index (text only, never raw HTML — see [the pipeline](../indexing/pipeline.md)) | Refreshed on re-crawl; **deleted with the generation on deindex or takedown** |
| Titles, headings, anchors | Same, usually short and factual | Same | Same |
| Published timestamps and author fields | Possibly personal data (a byline) | Index fields alongside the document | Indexed as document *content*, not user data; erased with the document |
| Robots/User-Agent identification | N/A | Crawler request header, config | Public crawler identity in `config/*.toml` |

The crawler stores **extracted text, never raw HTML**, which bounds the corpus's
exposure: what is kept is the smallest text that can still be searched and
snippets can excerpt.

## Takedown and erasure flow

1. A takedown request is recorded in the moderation queue (per-table retention
   documented; the queue is the only place a complainant's contact details ever
   exist, and it is field-level encrypted).
2. The document is marked no-index at the frontier level (crawl re-attempts avoid
   it) and excluded from the next generation build.
3. The **next successful generation** no longer contains it; the old generation
   retains it only within the bounded generation history (latest five kept), all
   within the 35-day encrypted-backup ceiling.
4. Queue verification: a build's audit confirms the URL did not appear in the new
   generation. The operator page
   [`runbooks.md`](../operations/runbooks.md#LynxTakedownQueueAged) treats an aged
   backlog item as an SLO miss.

Erasure under regulation (an "amnesia" request to remove the *whole* host,
for example) takes the same path with a host-scoped exclusion instead of a
document-scoped one. Grounded erasure is honest: it can confirm what LYNX stores,
because what LYNX stores is small and named — see
[data-minimisation](./data-minimisation.md).

## Statements of legal reach that this document deliberately avoids

LICENSE text, jurisdiction disclaimers, and DPA terms are not engineering
artifacts; they are handled where the law needs them to be. What an engineer or
operator needs to *do* in this repository is here and in
[PRIVACY.md](../../PRIVACY.md); nothing on this page replaces legal advice.
