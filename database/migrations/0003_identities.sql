-- 0003_identities.sql
-- +migrate:expand
--
-- Purpose : operator identity + roles, and developer API keys.
-- Duration: < 1 s.
-- Plans   : none invalidated.
-- Indexes : none; no concurrent build required.
--
-- Personal data: `admin_user.email_ciphertext` and `api_key.owner_email_ciphertext`
-- are field-level encrypted; only the HMAC lookup hash is queryable, and it is not
-- reversible. Retention class: governance. There is no end-user account: search
-- needs no identity, so there is no `user` table anywhere in this schema.

CREATE TABLE IF NOT EXISTS admin_user (
  id                uuid PRIMARY KEY DEFAULT uuidv7(),
  display_name      text NOT NULL,
  sso_subject       text UNIQUE,          -- IdP subject when SSO is configured
  email_ciphertext  bytea,                -- field-level encrypted
  email_lookup_hash bytea UNIQUE,         -- HMAC for lookup, never reversible
  disabled          boolean NOT NULL DEFAULT false,
  last_login_at     timestamptz,
  created_at        timestamptz NOT NULL DEFAULT now(),
  updated_at        timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS admin_role (
  id          uuid PRIMARY KEY DEFAULT uuidv7(),
  name        text NOT NULL UNIQUE,
  description text NOT NULL,
  permissions text[] NOT NULL DEFAULT '{}',
  created_at  timestamptz NOT NULL DEFAULT now()
);

-- Many-to-many: one operator may hold several roles, one role several operators.
CREATE TABLE IF NOT EXISTS admin_user_role (
  admin_user_id uuid NOT NULL REFERENCES admin_user(id) ON DELETE CASCADE,
  admin_role_id uuid NOT NULL REFERENCES admin_role(id) ON DELETE CASCADE,
  granted_at    timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (admin_user_id, admin_role_id)
);

-- A key is stored only as an HMAC of the secret. The plaintext exists once, at
-- creation, and is never recoverable from this table.
CREATE TABLE IF NOT EXISTS api_key (
  id                      uuid PRIMARY KEY DEFAULT uuidv7(),
  owner_label             text NOT NULL,
  owner_email_ciphertext  bytea,
  owner_email_lookup_hash bytea,
  key_prefix              text NOT NULL,          -- e.g. 'lnx_' + first 8 chars
  key_hash                bytea NOT NULL UNIQUE,  -- HMAC-SHA256 of the secret
  rate_limit_per_min      integer NOT NULL DEFAULT 300,
  created_at              timestamptz NOT NULL DEFAULT now(),
  last_used_at            timestamptz,
  expires_at              timestamptz,
  revoked_at              timestamptz
);
CREATE INDEX IF NOT EXISTS api_key_prefix_idx ON api_key (key_prefix);

CREATE TABLE IF NOT EXISTS api_key_scope (
  api_key_id uuid NOT NULL REFERENCES api_key(id) ON DELETE CASCADE,
  scope      text NOT NULL,
  PRIMARY KEY (api_key_id, scope)
);

DROP TRIGGER IF EXISTS admin_user_set_updated_at ON admin_user;
CREATE TRIGGER admin_user_set_updated_at
  BEFORE UPDATE ON admin_user
  FOR EACH ROW EXECUTE FUNCTION set_updated_at();
