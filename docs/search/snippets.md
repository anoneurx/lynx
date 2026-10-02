# Snippets and summaries

A snippet is the only part of a result a user reads before deciding. It has to be accurate,
useful, and short, which is a harder problem than it looks.

## Snippet selection

The best passage is chosen from the parsed content blocks, not by scanning raw HTML.

```rust
fn select_passage(doc: &ParsedDoc, plan: &QueryPlan) -> Snippet {
    // 1. candidate passages: text blocks split at heading and paragraph boundaries
    // 2. score each passage: term coverage, term proximity, position penalty, length fit
    // 3. prefer a passage that starts at a heading boundary (context for the reader)
    // 4. expand the chosen passage to whole sentences, never cutting mid-word
}
```

Passage scoring:

```text
coverage   = matched_query_terms / query_terms
proximity  = mean pairwise distance between matched term positions
position   = 1.0 at the document start, decaying to 0.4 deep in the body
length_fit = penalty if outside 120–320 characters

passage_score = 1.00·coverage + 0.30·proximity_fit + 0.15·position + 0.10·length_fit
```

`coverage` dominates, and it is not renormalised per passage: a passage containing two of four
query terms does not beat a passage containing all four because it is well positioned. A
snippet that reads beautifully and omits half the query is worse than a slightly clumsy one
that answers the question.

## Formatting

```text
1. passages are re-segmented into sentences at parse time and stored as sentence offsets
2. term matches are wrapped with <em> AFTER escaping, never before
3. a match spanning a sentence boundary is truncated at the first complete sentence
4. trailing fragments are trimmed at a word boundary with no ellipsis mid-word
5. length is enforced at 320 characters, cutting at the last sentence or clause
6. leading boilerplate ("Skip to content", nav lists) is excluded by block type
```

Two failure modes this avoids:

- **Highlighting injected markup.** Terms are escaped first, so a query of
  `<script>alert(1)</script>` becomes visible text, never an element. This is a correctness
  requirement, not a formatting preference.
- **Sentence fragments.** Snippets cut on sentence boundaries. A snippet starting mid-sentence
  tells the user nothing about what precedes it, which is most of what they need.

## Structured summaries

For pages with structured data, the snippet is a definition list rather than prose:

```json
{ "name": "tokio",
  "properties": [ { "label": "version",   "value": "1.42" },
                  { "label": "license",   "value": "MIT" } ] }
```

Rendered as key-value lines above the prose snippet. Sources are limited to data the publisher
declared as structured content — `Product`, `SoftwareApplication`, `Recipe`, `FAQPage`,
`Article`, `Organization` — and rendered as text, never as a card with images, prices, or
buttons. A summary that looks like an advertisement is not a summary.

## Entity disambiguation

When a query names an entity with a structured match, the snippet leads with the disambiguating
attribute rather than repeating the title:

```text
query:  rust lang
result: Rust (programming language) — systems language, edition 2021
        not the tungsten alloy; the infobox values come from a Person/Product match
```

The rule is narrow: disambiguation applies only when a structured entity of the expected type
matches the query term. Otherwise the first passage is used. Guessing about which of several
things a word refers to is how a snippet ends up confidently wrong.

## Missing and degraded content

| Case | Behaviour |
| ---- | --------- |
| No text blocks (image-only, PDF without text) | Title plus the site's own description; a `no_text` marker for operators |
| Content truncated at parse limit | Snippet drawn from what was parsed, with a notice |
| All query terms absent from the body | Show the document's leading passage rather than an empty snippet |
| Encrypted or paywalled content | Show the metadata the publisher provided; no attempt to bypass |

The last row is a hard constraint. LYNX indexes what a server gives to a compliant client. No
cookie replay, no paywall circumvention, no user-agent tricks to reach content behind a wall.

## Latency

```text
p50  6 ms      p95  20 ms
```

Sentence segmentation and term positions are computed at index time, so selection at query
time is scoring over pre-segmented passages with no re-parsing. Snippet generation is the
second-largest term in the p95 budget after retrieval, and the reason for the
[streaming](README.md) design: results can be sent as soon as they are ranked, with snippets
arriving immediately behind, so snippet latency is nearly invisible.

## Testing

- **Property tests:** snippet length always within bounds; highlighting always wraps escaped
  text; no snippet starts or ends mid-word; never contains an unescaped `<`.
- **Injection fixtures:** queries containing `<`, `>`, `&`, `"`, `</em>`, and a full script
  tag, each asserted to render as text.
- **Selection tests:** for a fixed fixture document and query, the expected passage is chosen;
  a passage with higher coverage beats a better-positioned passage with lower coverage.
- **Truncation tests:** documents whose first relevant passage is at the very start, very end,
  and beyond the parse limit.
- **Regression:** a snapshot of rendered snippets for the golden query set, reviewed on
  change — a snippet change is user-visible even when the ranking is unchanged.