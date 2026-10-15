-- 0002_privacy_foundation.sql
-- +migrate:expand
--
-- Purpose : retention policy and the administrative audit trail -- the tables that
--           make the privacy claims enforceable instead of aspirational.
-- Duration: < 1 s.
-- Plans   : none invalidated.
-- Indexes : none; no concurrent build required.
--
-- No table here can hold a search query: there is no column in which a query could
-- be stored. `admin_audit_log.admin_user_id` is a logical reference to
-- `admin_user` (created in 0003) rather than a hard FK, so an action that must be
-- recorded can be recorded even mid-migration of the identity plane.

CREATE TYPE retention_class AS ENUM (
  'crawl_permanent',  -- life of the URL in the index
  'crawl_temporary',  -- operational history, 30-90 days
  'governance',       -- legal / anti-abuse records
  'aggregate',        -- metrics rollups
  'no_user_data'      -- contains no personal data by construction
);

-- One row per retention class. A table that cannot name its class here does not
-- get created. `retain_days = NULL` means retained indefinitely.
CREATE TABLE IF NOT EXISTS retention_policy (
  class       retention_class PRIMARY KEY,
  retain_days integer,
  description text NOT NULL,
  created_at  timestamptz NOT NULL DEFAULT now(),
  updated_at  timestamptz NOT NULL DEFAULT now()
);

DROP TRIGGER IF EXISTS retention_policy_set_updated_at ON retention_policy;
CREATE TRIGGER retention_policy_set_updated_at
  BEFORE UPDATE ON retention_policy
  FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Append-only. There is deliberately no update or delete trigger: the record of an
-- administrative action must not be editable by the party that took it.
CREATE TABLE IF NOT EXISTS admin_audit_log (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  admin_user_id uuid,                 -- logical ref to admin_user(id); see header
  actor_label   text NOT NULL,        -- copied label; survives user deletion
  action        text NOT NULL,
  target_type   text,
  target_id     text,
  detail        jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS admin_audit_log_actor_idx
  ON admin_audit_log (admin_user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS admin_audit_log_target_idx
  ON admin_audit_log (target_type, target_id);
