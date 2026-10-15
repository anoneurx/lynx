# Governance (`0002_privacy_foundation.sql`, `0008_governance.sql`)

The audit trail and the two tables that may lawfully contain personal data. These
are the only tables in the schema with a personal-data column, and both store it
field-level encrypted with an HMAC lookup hash as the only queryable form.

## `retention_policy` (`0002`)

One row per `retention_class`, seeded in `database/seeds/seed.sql`. A retention job
reads `retain_days`; `NULL` means retained indefinitely. A class that is not
represented here is a bug, not a default.

## `admin_audit_log` (`0002`)

Append-only. There is deliberately no update or delete trigger: the record of an
administrative action must not be editable by the party that took it.
`admin_user_id` is a logical reference (no FK) with a copied `actor_label`, so the
trail survives identity changes. `action`, `target_type`, `target_id`, and a
`detail` `jsonb` payload are recorded.

## `abuse_report` (`0008`)

| Column | Type | Notes |
| ------ | ---- | ----- |
| `id` | `uuid` PK | |
| `category` | `text` | `spam`\|`malware`\|`harassment`\|`other` |
| `reporter_contact_ciphertext` | `bytea` | field-level encrypted |
| `reporter_contact_hash` | `bytea` | HMAC for dedup; never reversible |
| `target_url_id`, `target_domain_id` | `uuid` | what the report concerns |
| `detail_ciphertext` | `bytea` | encrypted free-text |
| `status` | `text` | `open`\|`reviewing`\|`closed` |
| `created_at`, `reviewed_at`, `reviewed_by`, `resolution` | | review workflow |

Retention: `governance`. Erasure of a reporter's contact is a purge of these two
columns, leaving the anonymised record.

## `takedown_request` (`0008`)

Written **before** removal. The record is the durable proof that a deletion was
requested, so a replay of the crawl cannot resurrect the document.

| Column | Type | Notes |
| ------ | ---- | ----- |
| `id` | `uuid` PK | |
| `url_id`, `doc_id` | `uuid` | target |
| `reason` | `text` | `unindex`\|`takedown`\|`ban`\|`soft_404`\|`court_order` |
| `requested_by` | `text` | `robots`\|`meta`\|`operator`\|`legal`\|`dmca` |
| `applied_to` | `text[]` | generations it was removed from |
| `verified` | `boolean` | set only after absence is confirmed by querying the index |
| `created_at`, `verified_at` | | |

A deletion that is not verified has only been asked for; the `verified` flag is
what makes it a deletion.
