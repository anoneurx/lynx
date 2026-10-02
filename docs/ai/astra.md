# Astra — grounded generation

Astra is LYNX's summary generator. It is built to solve one specific problem: condensing the
top-3 search results into a coherent paragraph without inventing facts.

```text
Query + Top-3 Snippets ──▶ Prompt Template ──▶ Local LLM (temp=0.0) ──▶ Citation Validator ──▶ Output
```

## Grounding mechanism

1. **Context assembly:** The prompt receives the user query and the exact text of the top-3 ranked
   document snippets, each prefixed with an index `[1]`, `[2]`, `[3]`.
2. **Zero temperature:** `temperature = 0.0` and `top_p = 0.1` to minimise creativity and force
   deterministic adherence to the provided text.
3. **System prompt constraints:**
   - *You are a factual summariser.*
   - *Use ONLY the provided sources. Do not use outside knowledge.*
   - *Every claim must cite its source like [1].*
   - *If the sources do not answer the question, say so.*

## Citation validation layer

Generated text is not sent directly to the user. It passes through a deterministic post-processor:

```rust
fn validate_citations(output: &str, sources: &[Snippet]) -> Result<GroundedSummary, ValidationError> {
    // 1. parse all bracketed citation numbers [N]
    // 2. verify every sentence contains at least one valid citation [N] where N <= sources.len()
    // 3. strip any sentence lacking a citation or referring to a non-existent source index
    // 4. reconstruct the summary with verified sentences only
}
```

If validation strips more than 50% of the sentences, the entire summary is discarded and the
fallback message is displayed: `"The retrieved sources could not be reliably summarised."`

## Self-hosting requirements

Astra is designed to run on modest infrastructure:

- **Model:** Llama-3-8B-Instruct (4-bit or 8-bit quantization via vLLM or Ollama).
- **Hardware:** 1× NVIDIA T4 / RTX 3090 / Apple Silicon M-series (16GB+ RAM).
- **Latency:** ~40 ms time-to-first-token, ~120 ms total generation time for 150 tokens.
- **Isolation:** Runs in a separate container network with no outbound internet access.

## What Astra cannot do

| Forbidden | Why |
| --------- | --- |
| Call external APIs | Egress allowlist blocks it; air-gapped design |
| Access user queries or history | Queries are ephemeral and never passed to Astra except the active one |
| Use tool calling / function calling | No tools exist; it is a pure text-in, text-out condenser |
| Bypass Tantivy retrieval | It only sees what ranking selected |

## Testing

- **Grounding tests:** A test suite of queries where the correct answer requires facts present
  only in source [2], asserting source [2] is cited and no outside facts appear.
- **Hallucination test:** Injecting a false premise into the query; Astra must refuse or state
  the sources do not support it.
- **Citation stripping test:** A generated response with invalid citation indices `[9]` is cleaned
  or rejected.