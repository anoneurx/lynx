# `GET /api/v1/suggestions`

Typeahead, built from the index rather than from query logs
([../search/suggestions.md](../search/suggestions.md)). The privacy property is structural: there
is no log to build it from.

## Request

| Parameter | Type | Default | Notes |
| --------- | ---- | ------- | ----- |
| `q` | string | required | The current prefix. Min 2 characters, max 128 bytes |
| `limit` | int | 8 | 1–10 |
| `scope` | enum | `all` | `all`, `titles`, `terms`, `related` |
| `interface_lang` | string | from `Accept-Language` | Selects the dictionary |

## Response

```json
{
  "query": { "prefix": "rust tra", "language": "en" },
  "suggestions": [
    { "text": "rust traits",     "kind": "terms",   "prior": 0.81, "doc_count": 8421 },
    { "text": "rust tracing",    "kind": "terms",   "prior": 0.74, "doc_count": 6102 },
    { "text": "rust training",   "kind": "terms",   "prior": 0.42, "doc_count": 1190 },
    { "text": "Rust trace macros", "kind": "titles", "prior": 0.39, "doc_count": 87 }
  ],
  "related": [
    { "text": "rust async", "doc_count": 3140 },
    { "text": "cargo workspace", "doc_count": 2980 }
  ],
  "meta": { "took_ms": 4, "index_generation": "gen-2026-02-17T04:00:00Z" }
}
```

| Field | Meaning |
| ----- | ------- |
| `text` | The full suggested string, ready to replace the input |
| `kind` | `terms`, `titles`, `terms`, `popular`, or `related` |
| `prior` | Corpus frequency prior, 0…1. **Not** query frequency, which does not exist |
| `doc_count` | Corpus support: how many documents contain the string |

The `doc_count` field is there so a client can distinguish "frequent in the corpus" from
"vaguely plausible". It also makes the index-derived nature of the suggestions visible rather
than implied.

## Empty and short prefixes

| `q` length | Response |
| ---------- | -------- |
| 0–1 | 200 with `suggestions: []`. No error; the client debounces and should not call |
| 2+ | Normal response |
| No prefix match | 200 with `suggestions: []` |

An empty suggestion list is a normal answer, not a failure. A 4xx here would make clients treat
an ordinary gap as an outage.

## Ranking

```text
score = 0.40·corpus_frequency_prior
      + 0.25·title_match_bonus
      + 0.20·co_occurrence_prior
      + 0.15·length_prior
```

Every component is derived from index statistics. There is no term from a user, which is why
`corpus_frequency_prior` is document frequency and not popularity-of-searching.

## What is never suggested

| Never | Mechanism |
| ----- | --------- |
| Anything from a query log | No logs exist |
| Content from `noindex` or banned domains | Filtered at the source, in the dictionary build |
| Strings a user submitted | No submission path exists |
| More than 4 words | Truncated |
| Adult, violence, or drug content | The crawler refused those pages before indexing |

The last row is a consequence of the crawler rather than a blocklist, which is the structural
point: the typeahead cannot suggest content the index does not contain.

## Privacy properties

| Property | How |
| -------- | --- |
| No retention | The handler writes nothing. The prefix is a request parameter and is not stored |
| No per-user state | Identical responses for every user; no session, no cookie |
| No third party | Served by LYNX; no external suggestion service |
| No keystroke leakage | The client sends the current prefix only, after an 80 ms debounce |
| Test-enforced | The handler's only persistence capability is the response cache, keyed by HMAC |

The last row is worth keeping as an invariant: the suggestions handler has no write access to
any durable store, so a future change that adds one fails to compile or fails the privacy test.

## Rate limits

More permissive than search, because typeahead fires per keystroke:

```text
anonymous   120 requests/minute per bucketed IP
with key    600 requests/minute
minimum     2 characters   — below this the request is not worth making
```

## Caching

```text
Cache-Control: public, max-age=300
```

The internal cache uses the same HMAC-keyed plan derivation as search, and the TTL is longer
because a prefix's completion set changes slowly.

## Accessibility

```text
role="combobox"     on the input, with aria-expanded and aria-controls
role="listbox"      on the suggestions container
role="option"       on each suggestion, with aria-selected
aria-autocomplete="list"
aria-activedescendant  pointing at the active option
```

| Key | Action |
| --- | ------ |
| `↓` / `↑` | Move the selection |
| `Enter` | Accept the selection |
| `Escape` | Dismiss, leaving the query unchanged |
| `Tab` | Close and move focus, without accepting |

The client's debounce is 80 ms and is **not** cancelled on `ArrowDown`. Cancelling it on every
keystroke is a common bug that makes the list stutter under keyboard use.

Without JavaScript the endpoint still works: the suggestion list renders below the input and is
reachable in document order, so a client can implement a plain `<select>` fallback.

## Related

- [../search/suggestions.md](../search/suggestions.md) — how suggestions are produced
- [../search/cache.md](../search/cache.md) — cache-key derivation
- [../privacy/privacy-model.md](../privacy/privacy-model.md) — the no-log commitment
- [openapi.yaml](./openapi.yaml) — the contract