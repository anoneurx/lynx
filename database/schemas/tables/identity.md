# Identity (`0003_identities.sql`)

Operator identity and developer API keys. There is no end-user identity anywhere in
the schema: search requires no account, so there is no `user`, `session`, or
`profile` table to leak.

## `admin_user`

| Column | Type | Notes |
| ------ | ---- | ----- |
| `id` | `uuid` PK | `uuidv7()` |
| `display_name` | `text` | |
| `sso_subject` | `text` UNIQUE | IdP subject; `NULL` for break-glass accounts |
| `email_ciphertext` | `bytea` | field-level encrypted; never logged |
| `email_lookup_hash` | `bytea` UNIQUE | HMAC for lookup; not reversible |
| `disabled` | `boolean` | soft disable instead of delete |
| `last_login_at` | `timestamptz` | |
| `created_at`, `updated_at` | `timestamptz` | `updated_at` by trigger |

Retention: `governance`. Not deletable while referenced by `admin_audit_log`.

## `admin_role`, `admin_user_role`

A role is a named set of permissions (`text[]`). `admin_user_role` is the
many-to-many join with explicit `granted_at`; both sides cascade on delete.

## `api_key`, `api_key_scope`

| Column | Type | Notes |
| ------ | ---- | ----- |
| `id` | `uuid` PK | |
| `owner_label` | `text` | human label, not personal data |
| `owner_email_ciphertext` | `bytea` | optional; encrypted |
| `owner_email_lookup_hash` | `bytea` | optional; HMAC |
| `key_prefix` | `text` | e.g. `lnx_` + first 8 chars, for lookup in logs |
| `key_hash` | `bytea` UNIQUE | HMAC-SHA256 of the secret; the secret is never stored |
| `rate_limit_per_min` | `integer` | default 300 |
| `created_at`, `last_used_at`, `expires_at`, `revoked_at` | `timestamptz` | revocation is soft |

`api_key_scope` holds the granted scopes so a key can be least-privilege by
construction. The plaintext key exists exactly once, at creation.

Personal data in this file: `email_ciphertext` / `owner_email_ciphertext`, both
present in `docs/privacy/data-minimisation.md`.
