//! From 0.1.5943's control schema, which 0.1.5947 shipped too: a provider
//! entry has share keys, none on every server until its owner makes one.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5943);

const CONTROL_FROM_0_1_5943: &str = "
CREATE TABLE provider_share_keys (
  token_hash  TEXT PRIMARY KEY,
  id          TEXT NOT NULL UNIQUE,
  provider_id TEXT NOT NULL REFERENCES providers (id) ON DELETE CASCADE,
  created_by  TEXT NOT NULL REFERENCES users (id),
  created_at  INTEGER NOT NULL,
  used_by     TEXT,
  used_at     INTEGER,
  CHECK ((used_by IS NULL) = (used_at IS NULL))
) STRICT;
CREATE INDEX provider_share_keys_provider ON provider_share_keys (provider_id, created_at);
";
