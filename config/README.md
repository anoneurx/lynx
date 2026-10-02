# LYNX configuration

Layered TOML. **No secrets in any file in this directory** — values are references only.

| File | Purpose |
| ---- | ------- |
| [`example.toml`](./example.toml) | every key, with comments, defaults documented |
| [`development.toml`](./development.toml) | local development: small budgets, verbose, relaxed limits |
| [`production.toml`](./production.toml) | production defaults and the constraints the validator enforces |

Layering and validation: [DEVELOPMENT.md](../DEVELOPMENT.md),
[docs/configuration.md](../docs/configuration.md).

Precedence: defaults < file < `LYNX_*` environment < (dev only) `--override` flags < secret store.

## Category namespaces

`crawler` · `index` · `ranking` · `database` · `cache` · `security` · `privacy` · `api` ·
`observability` · `ai` · `admin`

## Sensitive keys

Keys that resolve to secrets accept a reference form and never a value:

```toml
[database]
password = { secret_ref = "lynx/production/postgres/app" }

[ai]
api_key = { secret_ref = "lynx/production/ai/provider" }
```

Loading a file that contains a literal secret in a secret-typed key is a startup error, not
a warning. `make check-config` enforces this across the repository.

## Changing configuration that affects ranking

Weights live under `[ranking.signals]` and are **versioned**. Changing a weight requires
bumping `ranking.config_version`, which forces a re-index snapshot so that stored quality
features and served scores stay attributable. See
[docs/ranking/overview.md](../docs/ranking/overview.md).

## Changing configuration that affects privacy

Anything under `[privacy]` that weakens the model requires an ADR — see
[PRIVACY.md §8](../PRIVACY.md#8-configuration-that-can-weaken-this-model).