-- 0004_crawl_plane_core.sql
-- +migrate:expand
--
-- Purpose : the crawl graph -- domain, host, url -- and the frontier queue.
-- Duration: < 1 s on an empty database; partitions are created empty.
-- Plans   : none invalidated (initial creation).
-- Indexes : leading indexes are created with the tables; no concurrent build.
--
-- Hot path: the claim query is a partial index scan over `crawl_frontier`. The
-- conditions that keep it fast are exactly the ones a later schema change would
-- quietly break; see docs/database/schema.md, "Index maintenance".

CREATE TYPE crawl_state AS ENUM ('pending','ready','claimed','done','denied','banned');
CREATE TYPE discovery_source AS ENUM ('outlink','sitemap','feed','seed','operator');

-- The registrable domain: the unit of authority and of crawl budget.
CREATE TABLE IF NOT EXISTS domain (
  id         uuid PRIMARY KEY DEFAULT uuidv7(),
  name       text NOT NULL UNIQUE,       -- registrable domain, lowercased punycode
  first_seen timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- A specific hostname. Rules (robots, TLS, DNS) are per host; budget is per domain,
-- so a parked domain cannot inherit a subdomain's allowance.
CREATE TABLE IF NOT EXISTS host (
  id                   uuid PRIMARY KEY DEFAULT uuidv7(),
  domain_id            uuid NOT NULL REFERENCES domain(id),
  name                 text NOT NULL UNIQUE,   -- full hostname, lowercased punycode

  resolved_ip          inet,                    -- SSRF-validated address
  ip_verified_at       timestamptz,

  -- politeness budget, keyed by registrable domain (enforced in the claim query)
  rate_limit_seconds   integer NOT NULL DEFAULT 5,
  requests_in_window   integer NOT NULL DEFAULT 0,
  window_started_at    timestamptz,
  next_request_at      timestamptz NOT NULL DEFAULT now(),
  concurrency_limit    integer NOT NULL DEFAULT 1,

  -- observed behaviour
  robots_fetched_at    timestamptz,
  robots_status        integer,
  last_success_at      timestamptz,
  last_failure_at      timestamptz,
  consecutive_failures integer NOT NULL DEFAULT 0,
  failure_kind         text,

  -- content-type profile, for cost estimation
  avg_content_bytes    bigint,
  avg_parse_ms         integer,
  content_mix          jsonb,

  first_seen           timestamptz NOT NULL DEFAULT now(),
  updated_at           timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS host_domain_idx ON host (domain_id);
CREATE INDEX IF NOT EXISTS host_due_idx    ON host (next_request_at);

-- `url_hash` is unique on the *normalised* form, so //, trailing-slash, case, and
-- tracking-parameter variants collapse to one row. `normalised` records what was
-- changed, which is what makes an unexpected merge diagnosable rather than
-- mysterious. `fragment` is stored but never crawled.
CREATE TABLE IF NOT EXISTS url (
  id            uuid PRIMARY KEY DEFAULT uuidv7(),
  url_hash      bytea NOT NULL,        -- sha256(normalised_url)
  scheme        text NOT NULL,
  host_id       uuid NOT NULL REFERENCES host(id),
  path          text NOT NULL,
  query_string  text,
  fragment      text,

  canonical_url text NOT NULL,         -- after normalisation
  normalised    jsonb NOT NULL,        -- what canonicalisation changed
  first_seen    timestamptz NOT NULL DEFAULT now(),
  last_seen     timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS url_host_idx ON url (host_id);
CREATE INDEX IF NOT EXISTS url_path_idx ON url (host_id, path);
CREATE UNIQUE INDEX IF NOT EXISTS url_hash_idx ON url (url_hash);

-- The frontier is the crawler's memory and its throttle. Range-partitioned by
-- month so retention is DROP PARTITION rather than DELETE. `url_id`/`host_id` are
-- logical references (no FK) per specification.md section J rule 6, to keep write
-- contention predictable; integrity is enforced by the reconciliation job.
CREATE TABLE IF NOT EXISTS crawl_frontier (
  id            uuid NOT NULL DEFAULT uuidv7(),
  url_id        uuid NOT NULL,
  host_id       uuid NOT NULL,
  parent_url_id uuid,
  link_text     text,
  depth         smallint NOT NULL DEFAULT 0,

  priority      real NOT NULL DEFAULT 0,   -- sitemap > seed > link depth
  state         crawl_state NOT NULL DEFAULT 'pending',
  not_before    timestamptz NOT NULL DEFAULT now(),   -- retry backoff / politeness
  attempts      smallint NOT NULL DEFAULT 0,
  last_error    text,
  generation    bigint,
  discovery     discovery_source NOT NULL DEFAULT 'seed',

  claimed_by    text,
  claimed_at    timestamptz,
  fetched_at    timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

CREATE INDEX IF NOT EXISTS crawl_frontier_claim_idx
  ON crawl_frontier (state, not_before, priority DESC)
  WHERE state IN ('pending','ready');
CREATE INDEX IF NOT EXISTS crawl_frontier_reap_idx
  ON crawl_frontier (claimed_at) WHERE state = 'claimed';
CREATE INDEX IF NOT EXISTS crawl_frontier_url_idx  ON crawl_frontier (url_id);
CREATE INDEX IF NOT EXISTS crawl_frontier_host_idx ON crawl_frontier (host_id, state);

-- Catch-all partition so an insert never fails before the next month is created.
CREATE TABLE IF NOT EXISTS crawl_frontier_default PARTITION OF crawl_frontier DEFAULT;

-- Per-host, per-UTC-day budget counters. Budgets are enforced independently of
-- robots: a site that returns `Allow: /` gets no more budget than one that allows
-- nothing, and exhaustion drops the URL with a recorded reason rather than
-- retrying forever.
CREATE TABLE IF NOT EXISTS crawl_budget (
  host_id         uuid NOT NULL,
  day             date NOT NULL,
  pages_fetched   integer NOT NULL DEFAULT 0,
  bytes_fetched   bigint  NOT NULL DEFAULT 0,
  robots_delay_ms integer,
  cooldown_until  timestamptz,
  PRIMARY KEY (host_id, day)
);
CREATE INDEX IF NOT EXISTS crawl_budget_cooldown_idx
  ON crawl_budget (cooldown_until) WHERE cooldown_until IS NOT NULL;

DROP TRIGGER IF EXISTS domain_set_updated_at ON domain;
CREATE TRIGGER domain_set_updated_at BEFORE UPDATE ON domain
  FOR EACH ROW EXECUTE FUNCTION set_updated_at();
DROP TRIGGER IF EXISTS host_set_updated_at ON host;
CREATE TRIGGER host_set_updated_at BEFORE UPDATE ON host
  FOR EACH ROW EXECUTE FUNCTION set_updated_at();
DROP TRIGGER IF EXISTS url_set_updated_at ON url;
CREATE TRIGGER url_set_updated_at BEFORE UPDATE ON url
  FOR EACH ROW EXECUTE FUNCTION set_updated_at();
