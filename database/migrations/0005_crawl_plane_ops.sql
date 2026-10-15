-- 0005_crawl_plane_ops.sql
-- +migrate:expand
--
-- Purpose : crawl history and the ban / cooldown controls.
-- Duration: < 1 s at creation.
-- Plans   : none invalidated.
-- Indexes : created with the tables; no concurrent build.
--
-- `crawl_attempt` is diagnostic (30-day retention) and range-partitioned by month.
-- `outcome` distinguishes a technical failure from a policy decision, because
-- "we did not fetch it" and "the fetch failed" are different answers to the
-- question an operator is asking.

CREATE TYPE error_class AS ENUM (
  'timeout_connect','dns_failure','upstream_5xx','upstream_429','http_4xx',
  'content_too_large','unsupported_content_type','robots_disallow','safety_denied',
  'budget_exhausted','parse_failed','index_failed','redirect_loop','tls_failure'
);

CREATE TABLE IF NOT EXISTS crawl_attempt (
  id           bigint GENERATED ALWAYS AS IDENTITY,
  url_id       uuid NOT NULL,
  host_id      uuid NOT NULL,
  started_at   timestamptz NOT NULL DEFAULT now(),
  finished_at  timestamptz,
  status_code  integer,
  content_type text,
  bytes        integer,
  duration_ms  integer,
  outcome      text NOT NULL,   -- ok | error | blocked | skipped
  error_class  error_class,
  created_at   timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (id, started_at)
) PARTITION BY RANGE (started_at);

CREATE INDEX IF NOT EXISTS crawl_attempt_url_idx  ON crawl_attempt (url_id, started_at DESC);
CREATE INDEX IF NOT EXISTS crawl_attempt_host_idx ON crawl_attempt (host_id, started_at DESC);
CREATE TABLE IF NOT EXISTS crawl_attempt_default PARTITION OF crawl_attempt DEFAULT;

CREATE TYPE ban_scope AS ENUM ('url','host','domain');

-- Every ban stores a reason and an expiry, and every ban is visible in the admin
-- console. Bans are a budget mechanism as much as a safety one: a broken or
-- hostile host should not consume a worker's time indefinitely.
CREATE TABLE IF NOT EXISTS crawl_ban (
  id          uuid PRIMARY KEY DEFAULT uuidv7(),
  scope       ban_scope NOT NULL,
  url_id      uuid,
  host_id     uuid,
  domain_id   uuid,
  reason      text NOT NULL,
  created_by  text NOT NULL DEFAULT 'system',
  created_at  timestamptz NOT NULL DEFAULT now(),
  expires_at  timestamptz,       -- NULL = until the underlying condition changes
  reviewed_at timestamptz,
  reviewed_by uuid,
  CHECK (
    (scope = 'url'    AND url_id    IS NOT NULL) OR
    (scope = 'host'   AND host_id   IS NOT NULL) OR
    (scope = 'domain' AND domain_id IS NOT NULL)
  )
);
CREATE INDEX IF NOT EXISTS crawl_ban_host_idx   ON crawl_ban (host_id)   WHERE host_id   IS NOT NULL;
CREATE INDEX IF NOT EXISTS crawl_ban_domain_idx ON crawl_ban (domain_id) WHERE domain_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS crawl_ban_url_idx    ON crawl_ban (url_id)    WHERE url_id    IS NOT NULL;
CREATE INDEX IF NOT EXISTS crawl_ban_expiry_idx ON crawl_ban (expires_at) WHERE expires_at IS NOT NULL;

-- Daily rollup, retention class `aggregate`. Written by the crawler's flush job.
CREATE TABLE IF NOT EXISTS crawl_stat_daily (
  day             date NOT NULL,
  host_id         uuid NOT NULL,
  domain_id       uuid NOT NULL,
  pages_ok        integer NOT NULL DEFAULT 0,
  pages_error     integer NOT NULL DEFAULT 0,
  pages_denied    integer NOT NULL DEFAULT 0,
  bytes_fetched   bigint  NOT NULL DEFAULT 0,
  errors_by_class jsonb NOT NULL DEFAULT '{}'::jsonb,
  PRIMARY KEY (day, host_id)
);
CREATE INDEX IF NOT EXISTS crawl_stat_daily_domain_idx ON crawl_stat_daily (domain_id, day);
