# `packages/config` — configuration

Layered configuration with strict validation and secret indirection.

## Precedence (lowest to highest)

```text
built-in defaults
  < config/{environment}.toml
    < config/local.toml            (gitignored, developer overrides)
      < environment variables      (LYNX_*)
        < --config-override flags  (only in dev; refused in production)
          < secret store          (values referenced by secret_ref only)
```

## Rules

1. **Unknown keys are a hard error.** A typo'd config key silently using a default is how
   a crawler ends up hitting a network it should not. `deny_unknown_fields` everywhere.
2. **Secrets are references, never values.** A TOML file may contain
   `password = { secret_ref = "postgres/app" }`, never a password. Values arrive from the
   environment or a secret manager and are wrapped in `secrecy::SecretString`.
3. **Redaction on log.** The config type implements a `Redacted` view used by
   `GET /api/v1/admin/config` and by startup logs. It is impossible to serialise a secret
   value through those paths by accident.
4. **Environment-specific validation.** `production.toml` validation rejects: default
   secrets, `privacy.query_logging = true`, TLS below the configured minimum, admin
   binding without VPN enforcement, non-first-party CSP origins, weak TLS ciphers,
   `ai.enabled` without a pinned model id.
5. **Config fingerprint.** A hash of the effective config is logged at startup and stored
   on index snapshots and ranker output, so any result can be attributed to an exact
   configuration.

## Files

```text
packages/config/src/
├── lib.rs          loader, layering, validation entry point
├── schema.rs       typed config structs, one module per category
├── secret.rs       SecretRef resolution + redaction
├── validate.rs     environment-specific rules
└── fingerprint.rs  stable config hash
```

Category modules mirror `config/*.toml`:
`crawler`, `index`, `ranking`, `database`, `cache`, `security`, `privacy`, `api`,
`observability`, `ai`, `admin`. See [docs/configuration.md](../../docs/configuration.md).