# `services/ai` — Astra, the optional answer layer

**Binary:** `lynx-ai`. **Default:** `LYNX_AI_ENABLED=false`.
Full design: [docs/ai/](../../docs/ai/README.md).

Astra is a **strictly optional, strictly downstream** subsystem. Retrieval happens first
and independently; Astra summarises retrieved sources and cites them. If Astra is off,
disabled, over budget, or unverified, the user gets normal search results with no
degradation notice beyond a quiet absence.

## Non-negotiables

1. **Retrieval is primary.** Astra never generates a claim that is not attached to a
   retrieved document. If retrieval returns nothing useful, there is no answer.
2. **Crawled content is untrusted data, never instructions.** Web page text is wrapped in
   a delimited, explicitly-labelled data channel. Prompt-injection defences are specified
   in [docs/ai/prompt-injection.md](../../docs/ai/prompt-injection.md) and treated as a
   security control with its own threat entries.
3. **Fail closed.** Citation verification failure, schema violation, or budget exhaustion
   all result in "no answer", never in a partially-verified answer.
4. **Network isolation.** `lynx-ai` can reach the index reader, the cache, and exactly one
   configured model endpoint. It cannot reach the crawl plane or the admin plane.
5. **Bounded context.** Hard caps on tokens per source and in total, with per-source
   truncation that preserves the most query-relevant passages.
6. **No memory, no tools, no agent loop** in v0.1. One request in, one answer out. This is
   the single most effective injection mitigation available.

## Flow

```text
query ──▶ search index (unchanged path)
            │
            ▼
        top-K results (K = 8..12, already ranked and deduplicated)
            │
            ▼
        source extraction  (per-source passage selection, provenance offsets)
            │
            ▼
        prompt assembly     (system prompt + strict delimiters + numbered sources)
            │
            ▼
        model call          (single turn, structured output schema, no tools)
            │
            ▼
        verifier            (every sentence ↔ citation ↔ source span; numeric check)
            │ pass
            ▼
        answer + citations + fallback SERP always included
```

## Budget controls

| Control | Default | Purpose |
| ------- | ------- | ------- |
| `ai.max_source_tokens` | 6000 | Context ceiling |
| `ai.daily_token_budget` | 2,000,000 | Cost ceiling, enforced per prefix bucket |
| `ai.answer_cache_ttl` | 600 s | Repeat-query cost reduction; hashed key, no plaintext query |
| `ai.max_concurrency` | 8 | Prevents the AI layer from starving the search path |
| `ai.enabled` | `false` | Kill switch, honoured by the API at startup |

Every Astra request is **also** a normal search request. The search results are computed
and cached first; Astra is an additional response field. There is no path where Astra
failure suppresses search results.