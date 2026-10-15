-- 0008_governance.sql
-- +migrate:expand
--
-- Purpose : the two tables that may lawfully contain personal data.
-- Duration: < 1 s at creation.
-- Plans   : none invalidated.
-- Indexes : created with the tables; no concurrent build.
-- Retention: governance.
--
-- Personal data is field-level encrypted; only the HMAC lookup hash is queryable.
-- Abuse reports and takedown requests are the only personal-data-bearing tables in
-- the schema (see docs/threat-model.md, T30). Both are excluded from analytics by
-- construction: neither can hold a search query.

CREATE TABLE IF NOT EXISTS abuse_report (
  id                          uuid PRIMARY KEY DEFAULT uuidv7(),
  category                    text NOT NULL,   -- spam | malware | harassment | other
  reporter_contact_ciphertext bytea,           -- field-level encrypted
  reporter_contact_hash       bytea,           -- HMAC for dedup, never reversible
  target_url_id               uuid,
  target_domain_id            uuid,
  detail_ciphertext           bytea,
  status                      text NOT NULL DEFAULT 'open',  -- open | reviewing | closed
  created_at                  timestamptz NOT NULL DEFAULT now(),
  reviewed_at                 timestamptz,
  reviewed_by                 uuid,
  resolution                  text
);
CREATE INDEX IF NOT EXISTS abuse_report_status_idx ON abuse_report (status, created_at);
CREATE INDEX IF NOT EXISTS abuse_report_domain_idx
  ON abuse_report (target_domain_id) WHERE target_domain_id IS NOT NULL;

-- Written *before* removal. The tombstone is the durable record that a deletion
-- was requested, so a replay of the crawl cannot resurrect the document.
-- `verified` is set only after absence has been confirmed by querying the index,
-- because a deletion that is not verified has only been asked for.
CREATE TABLE IF NOT EXISTS takedown_request (
  id           uuid PRIMARY KEY DEFAULT uuidv7(),
  url_id       uuid,
  doc_id       uuid,
  reason       text NOT NULL,   -- unindex | takedown | ban | soft_404 | court_order
  requested_by text NOT NULL,   -- robots | meta | operator | legal | dmca
  applied_to   text[] NOT NULL DEFAULT '{}',   -- index generations removed from
  verified     boolean NOT NULL DEFAULT false,
  created_at   timestamptz NOT NULL DEFAULT now(),
  verified_at  timestamptz
);
CREATE INDEX IF NOT EXISTS takedown_request_unverified_idx
  ON takedown_request (verified) WHERE NOT verified;
CREATE INDEX IF NOT EXISTS takedown_request_url_idx
  ON takedown_request (url_id) WHERE url_id IS NOT NULL;
