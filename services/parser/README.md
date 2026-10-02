# `services/parser` — HTML to structured document

**Deployment:** v0.1 — library inside the `lynx-crawler` binary; becomes a batch worker
when parse CPU becomes the bottleneck. Full design: [docs/crawler/parser.md](../../docs/crawler/parser.md).

## Input: untrusted bytes

Parsed input is hostile by assumption. The parser must survive malformed markup,
adversarial nesting, entity expansion, and multi-gigabyte lie-about-`Content-Length`
bodies without panicking, hanging, or allocating unboundedly.

## Responsibilities

| Stage | Output |
| ----- | ------ |
| Decode | Bytes → text with a bounded, declared-charset-aware decoder; unknown charset falls back to UTF-8 with lossy replacement |
| Parse | Tokenised HTML tree (html5ever) with a node budget and depth cap |
| Extract visible text | `title`, `h1..h6`, paragraphs, list items, blockquote; skip `script`/`style`/`noscript`/`template` |
| Extract links | `<a href>`, `<link rel>`, `<img srcset>` → resolved absolute URLs, each re-normalised |
| Extract metadata | `<title>`, `<meta name=…>`, Open Graph, Twitter card, JSON-LD (parsed as inert data) |
| Canonicalise | `<link rel=canonical>` if it is same-origin-consistent, else the discovered URL |
| Classify | Language, content type, pagination hint (`rel=next`), likely-article flag |
| Extract dates | `article:published_time`, `dateModified`, JSON-LD `datePublished`, `<time datetime>`, heuristic from URL patterns |
| Sanitise | Strip control characters, zero-width and bidi-override characters, normalise Unicode (NFC) |
| Fingerprint | SimHash + MinHash sketches of the extracted text for duplicate detection |
| Classify intent | Instructional-injection patterns, keyword stuffing signals, doorway-page signals |

## Hard limits (all configurable, all enforced before allocation)

```text
max_body_bytes          5 MiB default
max_dom_nodes           2,000,000
max_dom_depth           512
max_links_per_document  50,000
max_text_bytes          2 MiB
max_parse_duration      2 s   (checked cooperatively; aborts mid-walk)
```

## Rules

- **We never store raw HTML.** Only extracted text and structured fields. This keeps the
  copyright surface and the XSS surface small. See
  [ADR-0016](../../docs/adr/0016-content-storage-policy.md).
- The parser output is a plain Rust struct (`ParsedDocument`) with no lifetime tied to the
  input, so the hostile buffer can be dropped immediately after extraction.
- JSON-LD and microdata are parsed into a bounded structure, never into a generic
  deserialiser that can be made to allocate without limit. Depth and length capped.
- Every parser field is fuzz-tested (`cargo-fuzz` targets: `fuzz_dom`, `fuzz_jsonld`,
  `fuzz_charset`, `fuzz_links`).