# ADR 0017: Sandboxed HTML Parsing

**Date:** 2024-06-15
**Status:** Accepted
**Deciders:** Security Team, Crawl Team
**Tags:** security, parsing, sandbox

## Context

Crawler fetches HTML from untrusted sources.
Parsing risks:
- **XXE** (XML external entities)
- **Billion laughs** (entity expansion)
- **ReDoS** (catastrophic backtracking in regex)
- **Memory exhaustion** (deep nesting, huge DOM)
- **XSS** (if parsed content ever rendered)

Must parse ~10M pages/day with minimal overhead.

## Decision

**Rust-native parser (`tlens`/`html5ever`) + resource limits + no scripting.**

### Parser Choice

- **`tlens`** (or `html5ever` + `markup5ever_rcdom`) → pure Rust, spec-compliant
- **No** `scraper` (uses `html5ever` but less control)
- **No** headless browser (Playwright/Puppeteer) for general crawl

### Resource Limits (per document)

| Limit | Value | Enforcement |
| ----- | ----- | ----------- |
| Max input size | 10 MiB | `reqwest::Body::max(10_485_760)` |
| Max DOM depth | 256 | Custom `TreeSink` impl |
| Max nodes | 500,000 | Counter in `TreeSink` |
| Parse timeout | 5 s | `tokio::time::timeout` |
| Max entity expansion | 10× | `tlens` default |

### Implementation

```rust
pub fn parse_html(bytes: &[u8]) -> Result<ParsedPage> {
    let opts = tlens::ParseOptions {
        max_depth: 256,
        max_nodes: 500_000,
        ..Default::default()
    };
    let dom = tlens::parse_with_options(bytes, opts)?;
    
    // Extract text (no script/style)
    let text = dom.text_content();
    let links = dom.select("a[href]").map(|e| e.attr("href")).collect();
    let title = dom.select("title").next().map(|e| e.text());
    let meta = extract_meta(&dom);
    
    Ok(ParsedPage { text, links, title, meta })
}
```

### No Script Execution

- **Never** execute JavaScript
- Strip `<script>`, `<style>`, `<noscript>`, `<iframe>`, `<object>`, `<embed>`
- `Content-Security-Policy` not applicable (we don't render)

### Fuzzing

- `cargo fuzz` target for `parse_html` (ADR 0018)
- Corpus seeded from real crawl (sanitized)
- Run in CI weekly

## Consequences

### Positive
- **Memory safe** (Rust) → no buffer overflows
- **No XXE** (HTML parser, not XML)
- **Bounded resources** → no OOM from malicious pages
- **Fast** (~2ms/page) → no browser overhead

### Negative
- **No JS-rendered content** → miss SPA content
- **Mitigation**: Separate "JS crawl" pipeline (Playwright) for known SPA domains (opt-in list)

## Alternatives Rejected

| Approach | Reason |
| -------- | ------ |
| Headless browser (all pages) | 100× slower, 10× memory, attack surface |
| `scraper` crate | Less control over limits |
| Custom regex parsing | Fragile, ReDoS risk |

## Related

- ADR 0010: Crawler Architecture
- ADR 0018: Fuzzing (parser target)
- Security Runbook RB-03: Parser Hardening