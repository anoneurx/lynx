-- seed.sql -- idempotent development seed data.
--
-- Safe to run repeatedly (`make db-seed`): every statement is ON CONFLICT DO
-- NOTHING and no row is ever updated. Contains no production content, no
-- third-party HTML, and no user data. See database/seeds/README.md.
--
-- Run after migrations:
--   psql "$DATABASE_URL" -f database/seeds/seed.sql

BEGIN;

-- Retention policy: one row per class. `retain_days = NULL` = indefinite.
INSERT INTO retention_policy (class, retain_days, description) VALUES
  ('crawl_permanent', NULL, 'Life of the URL in the index; until deindex or purge'),
  ('crawl_temporary',   90, 'Operational history (30-90 days)'),
  ('governance',       730, 'Legal / anti-abuse records'),
  ('aggregate',        396, 'Metrics rollups (13 months)'),
  ('no_user_data',    NULL, 'Contains no personal data by construction')
ON CONFLICT (class) DO NOTHING;

-- Built-in roles. Permissions are an allow-list, never inferred from the name.
INSERT INTO admin_role (name, description, permissions) VALUES
  ('viewer',   'Read-only admin console access',
     ARRAY['read']),
  ('operator', 'Operational actions: reindex, purge, ban',
     ARRAY['read','reindex','purge','ban']),
  ('admin',    'Full administrative control including keys and roles',
     ARRAY['read','reindex','purge','ban','manage_keys','manage_roles'])
ON CONFLICT (name) DO NOTHING;

-- Initial ranking configuration. Mirrors services/ranker/src/config.rs defaults;
-- a change to one is a change to both, in the same reviewed commit.
INSERT INTO ranking_config_version (version, config, notes, active) VALUES
  ('v0.1.0',
   '{
      "weights": {
        "lexical": 1.00, "freshness": 0.18, "quality": 0.12,
        "authority": 0.15, "context": 0.10, "spam_penalty": 0.40
      },
      "bm25": {
        "k1": 1.4, "k1_fused": 2.0, "b_title": 1.0, "b_headings": 1.0,
        "b_body": 0.5, "b_other": 0.75, "idf_floor": 0.0
      },
      "field_weights": {
        "title": 12.0, "headings": 6.0, "anchors": 4.0,
        "url": 3.0, "body": 2.0, "alt": 1.5
      },
      "freshness": {
        "half_life_days": 90.0, "crawled_half_life_days": 365.0,
        "w_published": 0.5, "w_modified": 0.3, "w_crawled": 0.2
      }
    }'::jsonb,
   'Initial LYNX ranking configuration',
   true)
ON CONFLICT (version) DO NOTHING;

-- One operator, linked to the operator role. Break-glass accounts carry no SSO
-- subject; this seeded one has one so re-seeding is idempotent.
INSERT INTO admin_user (display_name, sso_subject)
VALUES ('Seed Operator', 'seed|operator')
ON CONFLICT (sso_subject) DO NOTHING;

INSERT INTO admin_user_role (admin_user_id, admin_role_id)
SELECT u.id, r.id
FROM admin_user u
JOIN admin_role r ON r.name = 'operator'
WHERE u.sso_subject = 'seed|operator'
ON CONFLICT DO NOTHING;

-- A single synthetic host and the one example URL. RFC 2606 reserves
-- example.com for exactly this purpose.
INSERT INTO domain (name) VALUES ('example.com')
ON CONFLICT (name) DO NOTHING;

INSERT INTO host (domain_id, name, rate_limit_seconds)
SELECT d.id, 'www.example.com', 5
FROM domain d
WHERE d.name = 'example.com'
ON CONFLICT (name) DO NOTHING;

INSERT INTO url (url_hash, scheme, host_id, path, canonical_url, normalised)
SELECT
  digest('https://www.example.com/', 'sha256'),
  'https',
  h.id,
  '/',
  'https://www.example.com/',
  '{"changed": []}'::jsonb
FROM host h
WHERE h.name = 'www.example.com'
ON CONFLICT (url_hash) DO NOTHING;

-- Enqueue it once, as a seed discovery.
INSERT INTO crawl_frontier (url_id, host_id, priority, depth, state, not_before, discovery)
SELECT u.id, u.host_id, 1.0, 0, 'pending', now(), 'seed'
FROM url u
WHERE u.canonical_url = 'https://www.example.com/'
  AND NOT EXISTS (SELECT 1 FROM crawl_frontier f WHERE f.url_id = u.id);

COMMIT;
