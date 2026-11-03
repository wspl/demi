//! From 0.1.5928's control schema, which 0.1.5933 and 0.1.5939 shipped too:
//!
//! - An entry keeps the models its vendor refused to its key or routed to
//!   another model, which its list no longer offers
//!   (`crates/provider-registry/src/catalog.rs`).
//! - A provider entry has share keys, none on every server until its owner
//!   makes one.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5928);

const CONTROL_FROM_0_1_5928: &str = "
CREATE TABLE retired_models (
  provider_id TEXT NOT NULL REFERENCES providers (id) ON DELETE CASCADE,
  key_digest  TEXT NOT NULL,
  model_id    TEXT NOT NULL,
  answered_by TEXT,
  retired_at  INTEGER NOT NULL,
  PRIMARY KEY (provider_id, key_digest, model_id)
) STRICT;
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
