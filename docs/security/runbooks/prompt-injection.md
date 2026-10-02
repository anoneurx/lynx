# RB-06 · Suspected prompt injection

**Severity:** SEV2, escalating to SEV1 if a system instruction was subverted · **Owner:** AI
plus security

A crawled page containing text aimed at the model is not a novel attack. It is expected. What
matters is whether it achieved anything.

## First action

```bash
lynx-feature disable astra        # optional capability, zero user impact
```

Disable rather than investigate. The search plane is the product; the AI layer is an
enhancement, and the cost of disabling is far below the cost of a live injection while the
investigation runs.

## Determine the impact

| Question | How | Consequence |
| -------- | --- | ---------- |
| Was the injected text followed as an instruction? | Read the completed answer against the retrieved context | None: the model was manipulated, output is discarded |
| Did output escape the grounded format? | Check the citation validator | None: the answer is rejected |
| Did the output contain an instruction the user could follow? | Read the answer | **None: the output is text with no capability** |
| Was any tool invoked? | There are no tools | Cannot have happened |
| Was any network request made? | The AI plane has no egress | Cannot have happened |
| Was any file or index write made? | Read-only retrieval over existing docs | Cannot have happened |

The structural answer is that Astra has **no capabilities**: no tools, no fetch, no writes, no
network. So the worst achievable outcome from a successful injection is a badly-worded answer,
which the validator rejects anyway. This runbook exists to establish that, not because the
outcome is expected to be worse.

## Classify the payload

```text
direct          "ignore previous instructions and …"   in the page body
indirect        instructions in a comment, alt text, or metadata field
encoded         base64, homoglyphs, zero-width characters, split across elements
retrieval       placed to be retrieved for a specific query, i.e. aimed at LYNX specifically
cross-site      content intended for a model that has tools, not for a human
```

`retrieval` class deserves attention: it means someone specifically targeted LYNX's AI layer,
which makes it an attack rather than noise.

## Contain and mitigate

```text
1  Astra stays disabled until the mitigation ships
2  add the payload's normalised text to the retrieval exclusion filter
3  add a detection test: a fixture page with the payload must not change an answer
4  review whether the payload reached the context at all
```

The exclusion filter is a blunt instrument and this document does not claim otherwise: it
removes pages from AI retrieval on a heuristic, which has false positives. The deeper mitigation
is the structural one already in place — grounded generation with citation validation, and no
capabilities.

## Detection improvements

| Improvement | Rationale |
| ----------- | --------- |
| Normalise before matching | Homoglyph and zero-width variants defeat naive matching |
| Score context for injection patterns | Warn in the response metadata when a high score is detected |
| Maintain a payload corpus | Every reported payload becomes a permanent fixture |
| Monitor answer-rejection rate | A sudden rise indicates a new campaign |

The rejection-rate monitor is the general detector. It does not need to recognise any specific
payload to notice that something changed.

## Verification before re-enabling

```bash
# 1. the payload fixture produces a rejected or unchanged answer
lynx-eval injection --corpus fixtures/injection --expect-reject

# 2. citation validation passes on the fixture set
lynx-eval grounding --corpus fixtures/ai --min-citations 1

# 3. the capability set is still empty
lynx-feature list --plane astra | jq '[.capabilities[]] | length'   # expect 0
```

All three must pass. The third is the important one, because it verifies the structural
mitigation rather than a filter.

## Escalate

- Any capability invocation → impossible by design; investigate a code defect immediately
- Cross-site payloads suggesting the attacker knows LYNX's design → SEV2, and publish the
  finding in the threat model
- Repeated, targeted attempts → publish a mitigation note; responsible disclosure is welcome
  and [SECURITY.md](../../../../SECURITY.md) applies