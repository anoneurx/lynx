# Versioning

`/api/v1` is stable once shipped. This document defines what "stable" means, because a version
in a path that implies more than it delivers is worse than no versioning.

## Policy

| Guarantee | Meaning |
| --------- | ------- |
| `v1` is stable | No breaking change to an existing endpoint in `v1`, ever |
| Additive changes are not breaking | New optional response fields, new optional parameters, new endpoints |
| Breaking changes require `v2` | A new path prefix, alongside `v1` for at least one minor cycle |
| Deprecation is announced in `CHANGELOG.md` | At least one minor version before removal |
| The `Sunset` header is sent | With the removal date, on every response from a deprecated endpoint |
| The `Link` header points at the replacement | `rel="deprecation"` |

## Non-breaking additions

| Change | Breaking? |
| ------ | --------- |
| A new field in a response | No — clients ignore unknown fields |
| A new optional query parameter | No |
| A new error code | No — treat unknown codes as non-retryable |
| A new endpoint | No |
| A new optional request field | No |
| A new `signals` entry | No |

The rule that makes this safe is stated as a client obligation:

> Clients **must** ignore unknown response fields and **must** not require any response field
> except those marked required in the OpenAPI document.

Without that obligation, no addition is safe, and the only options are a version bump for every
feature or freezing the API.

## Breaking changes

| Change | Why it breaks |
| ------ | ------------- |
| Removing or renaming a response field | A documented client reads it |
| Changing a field's type | Parsers break silently |
| Making an optional parameter required | Existing requests fail |
| Changing a default that alters results | Same request, different results |
| Changing an error `code` meaning | Client branching breaks, and it may already be deployed |
| Removing an error code | A client may treat it as retryable |
| Changing rate-limit semantics | Clients adapt their retry behaviour |
| Tightening validation on an accepted input | A previously working request starts failing |
| Changing the ranking algorithm | **Deliberately not breaking, and deliberately visible** |

The last row deserves attention. Ranking changes alter results for the same query, which is a
behaviour change in the most visible sense. It is not treated as an API break, because ranking
quality is the product's purpose and freezing it would freeze the project. It is instead made
visible through `meta.ranking_config_version` and `meta.index_generation`, so a client — and a
user reporting a result — can identify the change. See [../ranking/explainability.md](../ranking/explainability.md).

## What is frozen in `v1`

| Frozen | Rationale |
| ------ | --------- |
| Endpoint paths and methods | The contract |
| Field names and types | The contract |
| Error `code` values and meanings | The contract |
| HTTP status mapping | The contract |
| Header names | The contract |
| Operator syntax | Users learn it; changing it breaks their muscle memory |

Operator syntax is the one that is easy to overlook. `site:` and `-` are not in the OpenAPI
document, but they are part of what a user of the search engine knows, so changing them is a
product change rather than an API change.

## Deprecation

```text
1  announce in CHANGELOG.md, with the alternative
2  emit  Sunset: <HTTP-date>; rel="deprecation"  and  Link: <…>; rel="deprecation"
3  log usage so the deprecation is measured, not assumed
4  remove only after the measured usage reaches zero and the minimum notice has elapsed
5  remove only in a major version
```

Step 3 is the part that is usually skipped and always regretted. A deprecation announced into
the void is not a deprecation, it is a note. Measuring usage is what turns an intention into a
plan, and it is possible here because endpoint usage can be counted per path without counting
queries.

## Supporting multiple versions

```text
v1 and v2 coexist
they share one handler core
the version selects a response-shaping layer only
```

Sharing the core matters: two independent implementations of a search endpoint would diverge in
ranking, in error handling, and in privacy behaviour, and the divergence would be invisible
until it caused an incident. A version selects how the response is shaped, not how the search is
performed.

## Version identification

```text
X-Lynx-Version        service version
X-Lynx-Index-Generation
X-Lynx-Ranking-Config
X-Request-Id
```

The index generation and ranking config version are the interesting pair: they are what makes a
result reproducible, so a report of "the results were wrong" can be tied to a specific index
state rather than to a moment in time.

## Client guidance

```text
Do:  ignore unknown fields
     treat unknown error codes as non-retryable
     surface corrections and notices to the user
     record the index generation when reporting a result
     re-read the OpenAPI document on version bumps, not on every deploy

Don't: parse error messages
     require a field unless the document marks it required
     depend on result ordering being stable across generations
     treat an empty result set as an error
```

The last prohibition is the most common client bug: treating "no matches" as a failure turns a
correct answer into an error, and produces error reports for queries that genuinely have no
results.

## Release coupling

| Change type | Requires |
| ----------- | -------- |
| Additive API change | Minor version, changelog entry |
| New endpoint | Minor version, OpenAPI update, changelog entry |
| Breaking change | Major version, new path prefix, `v1` retained |
| Deprecation | Minor version, changelog, `Sunset` header, usage measurement |
| Ranking change | No version change; `ranking_config_version` bump, documented in the changelog |

The last row is the deliberate distinction: a ranking change is documented loudly but does not
get its own version, because "the results are better" is the expected outcome of shipping, and a
version bump for it would train clients to expect breakage on every release.

## Related

- [errors.md](./errors.md) — the code stability policy
- [openapi.yaml](./openapi.yaml) — the contract
- [../../CHANGELOG.md](../../CHANGELOG.md) — deprecations are announced there
- [../ranking/explainability.md](../ranking/explainability.md) — why ranking changes are not breaks