-- 0001_extensions.sql
-- +migrate:expand
--
-- Purpose : extensions, shared helper functions, and the migration ledger.
-- Duration: < 1 s (CREATE EXTENSION on an empty database).
-- Plans   : none invalidated (initial creation).
-- Indexes : none; no concurrent build required.
--
-- PostgreSQL 16 target. `uuidv7()` became a builtin in PostgreSQL 18; the guard
-- below defines a compatible implementation only when the server lacks one, so
-- this migration is safe on 16 and 18 alike and never shadows the builtin.

CREATE EXTENSION IF NOT EXISTS pgcrypto;   -- gen_random_bytes, digest
CREATE EXTENSION IF NOT EXISTS pg_trgm;    -- trigram indexes for text columns
CREATE EXTENSION IF NOT EXISTS btree_gin;  -- GIN over scalar columns

-- Migration ledger. Written only by the migration runner. A checksum mismatch is
-- a startup failure, not a warning: the applied history is part of the audit trail.
CREATE TABLE IF NOT EXISTS schema_migrations (
  version    integer     PRIMARY KEY,
  name       text        NOT NULL,
  checksum   text        NOT NULL,
  applied_at timestamptz NOT NULL DEFAULT now()
);

-- RFC 9562 version-7 UUIDs: 48-bit big-endian unix milliseconds, then random bits
-- with the version and variant fields set. Time-ordered, so B-tree inserts stay
-- local and range scans on `first_seen` behave. Defined only when the server has
-- no uuidv7() of its own.
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_proc WHERE proname = 'uuidv7') THEN
    CREATE FUNCTION uuidv7() RETURNS uuid AS $fn$
    DECLARE
      ts  bytea := substring(
                     int8send((extract(epoch FROM clock_timestamp()) * 1000)::bigint)
                     FROM 3);
      rnd bytea := gen_random_bytes(10);
    BEGIN
      rnd := set_byte(rnd, 0, (get_byte(rnd, 0) & 15) | 112);  -- 0x70: version 7
      rnd := set_byte(rnd, 2, (get_byte(rnd, 2) & 63) | 128);  -- 0x80: RFC 4122 variant
      RETURN encode(ts || rnd, 'hex')::uuid;
    END;
    $fn$ LANGUAGE plpgsql VOLATILE;
  END IF;
END;
$$;

-- Maintains `updated_at` on mutable tables so application clocks never disagree
-- with the database's.
CREATE OR REPLACE FUNCTION set_updated_at() RETURNS trigger AS $$
BEGIN
  NEW.updated_at := now();
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;
