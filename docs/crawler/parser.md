# Parser

Input is hostile by assumption. The parser must survive malformed markup, adversarial
nesting, entity expansion, and absurd documents without panicking, hanging, or allocating
without bound.

## Pipeline

```text
bytes
 → bounded charset decode
 → HTML tokenisation (html5ever)
 → DOM walk with node/depth/budget limits
 → visible text extraction
 → link extraction and URL resolution
 → metadata extraction (title, meta, Open Graph, Twitter, JSON-LD, microdata)
 → canonicalisation
 → classification (language, dates, content type)
 → sanitisation (control chars, zero-width, bidi overrides, NFC)
 → fingerprints (SimHash, MinHash)
 → spam and injection feature extraction
 → ParsedDocument
```

## Hard limits

```text
max_body_bytes          5 MiB      (already enforced by the downloader)
max_dom_nodes           2,000,000
max_dom_depth           512
max_links_per_document  50,000
max_text_bytes          2 MiB
max_parse_duration      2 s         (cooperative abort mid-walk)
max_metadata_bytes      64 KiB
max_jsonld_nodes        5,000
```

Exceeding a limit is not a crash. It truncates and records a flag on the document
(`truncated`, `parse_limit_hit`), which becomes a quality signal in ranking rather than a
silent data loss.

## Text extraction

Included: `<title>`, `<h1>`–`<h6>`, `<p>`, `<li>`, `<blockquote>`, `<td>`, `<figcaption>`,
`<dt>`/`<dd>`, and text directly in block containers.

Excluded entirely: `<script>`, `<style>`, `<noscript>`, `<template>`, `<svg>` internals,
`<iframe>` contents, hidden elements (`display:none`, `hidden`, `aria-hidden`), and
comment content. Hidden text is a known spam technique; excluding it structurally is
better than trying to detect it.

Main-content extraction: rather than trusting the first `<p>` on a page, LYNX uses a
density heuristic — the subtree with the highest text-to-tag ratio and lowest link density
— plus `rel=canonical` and `article`/`main` landmark preference. `main_content_text` is
stored separately from `full_text` so ranking can weight them differently.

## Links

- `<a href>`, `<area href>`, `<link href>` (for `rel=next`, `rel=canonical`,
  `rel=alternate` hreflang), `<img src>`, `<script src>` (not followed, only recorded),
  `<iframe src>` (not followed).
- Every href is resolved against the document base, then passed through the **same
  canonicalisation as the frontier**, so the parser and the frontier can never disagree
  about what the same URL is.
- Schemes other than `http`/`https` are recorded as link *types* (`mailto`, `tel`, `ftp`,
  `javascript`) but never queued. `javascript:` and `data:` links are discarded entirely.
- Fragment-only links are dropped (same document).
- Duplicates are collapsed; the per-document link cap applies after deduplication.
- Anchor text is stored per link because anchor text is a ranking signal
  (`inanchor:` operator).

## Metadata

| Source | Fields |
| ------ | ------ |
| `<title>` | title, with length sanity checks and whitespace collapse |
| `<meta name=…>` | description, robots (and `noindex` honoured), keywords (ignored for ranking by design) |
| Open Graph | `og:title`, `og:description`, `og:url`, `og:site_name`, `og:type` |
| Twitter card | `twitter:title`, `twitter:description`, `twitter:image` |
| `<link rel=canonical>` | canonical URL, accepted only if same-host or cross-host-but-same-registrable-domain; otherwise the discovered URL wins |
| `<link rel=alternate hreflang>` | language/region alternates, used for `lang:` filtering and duplicate detection |
| JSON-LD `application/ld+json` | `headline`, `datePublished`, `dateModified`, `author`, `articleSection`, `isPartOf` |
| Microdata / RDFa | same fields, lower priority |
| `<time datetime>` | publication date candidates |
| `<html lang>` | language hint |

Canonicalisation rule worth stating: a `rel=canonical` pointing at a different host is
treated as a **hint**, not a command. Following cross-host canonicals is a known
canonical-hijacking vector, and the authority that would benefit from it is an attacker.
Same-host canonicals are honoured because they are the normal case.

## Dates

```text
priority  1  JSON-LD datePublished / dateModified
          2  meta property="article:published_time" / "article:modified_time"
          3  <time datetime> within an article container
          4  Open Graph article:published_time
          5  URL path patterns (/2026/01/14/, /2026-01-14/)
          6  nothing → unknown (never guessed)
```

When nothing is found, `published_at` is null and only `crawled_at` is available. Freshness
falls back to crawl recency, which is a weaker signal and is treated as such. Fabricating a
date from a filename would poison the freshness ranking permanently.

## Language detection

Primary: a fast character-n-gram classifier over the extracted text, trained on the
crawl corpus itself (no third-party training data, no licensing question). Secondary:
`<html lang>`, `Content-Language`, and hreflang alternates. Disagreement between detector
and declared language lowers the confidence field and is a quality signal, not an error.

Language selects the stopword list and the stemmer at index time. Multi-language pages are
split by block where the language changes, and stored as language sub-fields so a query in
one language does not match text in another.

## Sanitisation

Applied to every stored text field, in this order:

```text
1. strip C0/C1 control characters except tab and newline
2. strip Unicode format characters: zero-width (U+200B-200D, U+FEFF),
   bidi overrides and isolates (U+202A-202E, U+2066-2069), soft hyphen
3. normalise to NFC
4. collapse whitespace runs to single spaces (except in preformatted contexts)
5. fold homoglyph confusables in display-only fields (Cyriilic а → Latin a is NOT applied
   to indexed text — we index what it says and normalise at display)
```

Item 5's split matters: folding at index time would merge genuinely distinct words and
corrupt search, while not folding at all lets a phishing page look identical to a bank in a
snippet. So: index the real text, flag the confusion, and let ranking and display decide.

## Fingerprints and features

| Fingerprint | Use |
| ----------- | --- |
| SimHash (64-bit, shingled) | fast near-duplicate candidate lookup via banded LSH |
| MinHash (64 permutations) | near-duplicate verification and cluster assignment |
| Content length, link count, DOM depth | quality and trap signals |
| Text entropy, term distribution | keyword stuffing detection |
| Heading repetition | doorway/template-page detection |
| Template shingle overlap across domains | site-cluster detection |
| Time-of-day content variance | cloaking signal |
| Instruction-like phrases | prompt-injection flag for the AI layer |

All are cheap integer or float features computed once at parse time and stored with the
document. Ranking reads features; it never re-parses content.

## Output

`ParsedDocument` — a plain owned struct with no lifetime tied to the input bytes, so the
hostile buffer can be dropped immediately after extraction:

```rust
struct ParsedDocument {
    url_id, host_id, domain_id,
    discovered_url, canonical_url,
    status, content_type, charset,
    title, description, site_name,
    main_text, full_text, headings: Vec<Heading>,
    anchors: Vec<(String, String)>,        // (text, target_url)
    language, language_confidence,
    published_at, modified_at, discovered_at,
    simhash, minhash,
    features: DocFeatures,
    quality_flags: QualityFlags,           // truncated, js_required, thin, no_robots_meta, …
    parsed_at, parser_version,
}
```

No raw HTML field. No script content. No base64 blobs. See
[ADR-0016](../adr/0016-content-storage-policy.md).

## Fuzzing

`cargo-fuzz` targets: `fuzz_dom` (arbitrary markup), `fuzz_jsonld`, `fuzz_charset`,
`fuzz_links` (arbitrary hrefs), `fuzz_entities`. CI runs each for a bounded nightly
session and fails the build on any crash, hang, or out-of-memory.