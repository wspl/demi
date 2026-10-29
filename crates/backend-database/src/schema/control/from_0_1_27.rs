//! From 0.1.27's control schema to the next one.
//!
//! - The users' port forwards
//!   (`crates/backend-user-shard/src/shard/port_forwards.rs`) and exposed ports
//!   (`crates/backend-user-shard/src/shard/exposed_ports.rs`), none before.
//! - Each owner's entries have an order of their own, which the user sets by
//!   dragging Settings › Providers and generated media try them in, first the
//!   order they were added in
//!   (`crates/backend-user-shard/src/provider_settings.rs`).
//! - Preferences no longer choose an entry per vendor's images, which the order
//!   and the switches say instead, so a saved choice is dropped.
//! - An entry whose provider makes videos has a Video Generation switch, and one
//!   whose provider makes audio an Audio Generation switch, each on for every
//!   entry stored before.
//! - A usage ledger row may be a video's or an audio file's, with the seconds
//!   made and the cost the vendor reported
//!   (`crates/backend-database/src/usage.rs`), which SQLite's checks let in only
//!   on a table made again, its rows as they were with null for both.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_27);

const CONTROL_FROM_0_1_27: &str = "
CREATE TABLE port_forwards (
  id                TEXT PRIMARY KEY,
  user_id           TEXT NOT NULL REFERENCES users (id),
  name              TEXT NOT NULL,
  listen_device_id  TEXT NOT NULL REFERENCES devices (id),
  listen_port       INTEGER NOT NULL CHECK (listen_port BETWEEN 1 AND 65535),
  connect_device_id TEXT REFERENCES devices (id),
  connect_address   TEXT NOT NULL,
  enabled           INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  created_at        INTEGER NOT NULL,
  UNIQUE (user_id, name)
) STRICT;
CREATE INDEX port_forwards_of_user ON port_forwards (user_id, created_at);
CREATE TABLE exposed_ports (
  id         TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users (id),
  device_id  TEXT NOT NULL REFERENCES devices (id),
  port       INTEGER CHECK (port BETWEEN 1 AND 65535),
  label      TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  UNIQUE (device_id, port)
) STRICT;
CREATE INDEX exposed_ports_of_user ON exposed_ports (user_id, created_at);
ALTER TABLE providers ADD COLUMN position INTEGER NOT NULL DEFAULT 0 CHECK (position >= 0);
UPDATE providers SET position = (
  SELECT count(*) FROM providers AS earlier
  WHERE earlier.owner_user_id = providers.owner_user_id
    AND (earlier.created_at < providers.created_at
      OR earlier.created_at = providers.created_at AND earlier.rowid < providers.rowid)
);
DROP INDEX providers_owner;
DROP INDEX providers_one_subscription;
CREATE INDEX providers_owner ON providers (owner_user_id, position);
CREATE UNIQUE INDEX providers_one_subscription ON providers (owner_user_id, provider_type)
  WHERE credential_kind = 'subscription';
UPDATE user_preferences SET preferences = json_remove(preferences, '$.imageEntries')
  WHERE json_type(preferences, '$.imageEntries') IS NOT NULL;
ALTER TABLE providers ADD COLUMN video_generation INTEGER NOT NULL DEFAULT 1 CHECK (video_generation IN (0, 1));
ALTER TABLE providers ADD COLUMN audio_generation INTEGER NOT NULL DEFAULT 1 CHECK (audio_generation IN (0, 1));
CREATE TABLE usage_ledger_media (
  id                    TEXT PRIMARY KEY,
  user_id               TEXT NOT NULL REFERENCES users (id),
  conversation_id       TEXT NOT NULL,
  agent                 INTEGER CHECK (agent >= 0),
  turn_id               TEXT,
  purpose               TEXT CHECK (purpose IN ('turn', 'compaction', 'title', 'image', 'video', 'audio')),
  provider_id           TEXT NOT NULL,
  account_id            TEXT,
  model_id              TEXT NOT NULL,
  request_id            TEXT,
  vendor_id             TEXT,
  input_tokens          INTEGER,
  output_tokens         INTEGER,
  cache_read_tokens     INTEGER,
  cache_write_tokens    INTEGER,
  cache_write_1h_tokens INTEGER,
  outcome               TEXT,
  quota                 TEXT,
  started_at            INTEGER,
  ended_at              INTEGER NOT NULL,
  media_seconds         REAL CHECK (media_seconds >= 0),
  cost                  REAL CHECK (cost >= 0),
  cost_currency         TEXT CHECK (cost_currency IN ('usd', 'cny')),
  CHECK ((cost IS NULL) = (cost_currency IS NULL))
) STRICT;
INSERT INTO usage_ledger_media (id, user_id, conversation_id, agent, turn_id, purpose, provider_id, account_id,
    model_id, request_id, vendor_id, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
    cache_write_1h_tokens, outcome, quota, started_at, ended_at)
  SELECT * FROM usage_ledger ORDER BY rowid;
DROP TABLE usage_ledger;
ALTER TABLE usage_ledger_media RENAME TO usage_ledger;
CREATE INDEX usage_ledger_user ON usage_ledger (user_id, ended_at);
";
