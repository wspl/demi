//! From 0.1.5773's control schema to the next one: a usage ledger row may be
//! a request for the replies the user is likely to send after an answer
//! (`crates/agent-server/src/suggested_replies.rs`), which SQLite's check
//! lets in only on a table made again, its rows and indexes as they were.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5773);

const CONTROL_FROM_0_1_5773: &str = "
CREATE TABLE usage_ledger_next (
  id                    TEXT PRIMARY KEY,
  user_id               TEXT NOT NULL REFERENCES users (id),
  conversation_id       TEXT NOT NULL,
  agent                 INTEGER CHECK (agent >= 0),
  turn_id               TEXT,
  purpose               TEXT CHECK (purpose IN ('turn', 'compaction', 'title', 'suggested_replies', 'image', 'video', 'audio')),
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
  api_price_input       REAL CHECK (api_price_input >= 0),
  api_price_cache_write REAL CHECK (api_price_cache_write >= 0),
  api_price_cache_read  REAL CHECK (api_price_cache_read >= 0),
  api_price_output      REAL CHECK (api_price_output >= 0),
  system_hash           TEXT,
  tools_hash            TEXT,
  call_number           INTEGER CHECK (call_number >= 1),
  after_compaction      INTEGER CHECK (after_compaction IN (0, 1)),
  miss                  TEXT CHECK (miss IN ('model_changed', 'tools_changed', 'system_prompt_changed', 'cache_expired', 'not_cached')),
  CHECK ((cost IS NULL) = (cost_currency IS NULL))
) STRICT;
INSERT INTO usage_ledger_next SELECT * FROM usage_ledger ORDER BY rowid;
DROP TABLE usage_ledger;
ALTER TABLE usage_ledger_next RENAME TO usage_ledger;
CREATE INDEX usage_ledger_user ON usage_ledger (user_id, ended_at);
CREATE INDEX usage_ledger_conversation ON usage_ledger (conversation_id, ended_at);
CREATE INDEX usage_ledger_agent ON usage_ledger (conversation_id, agent);
CREATE INDEX usage_ledger_cache_writes ON usage_ledger (conversation_id, agent, model_id)
  WHERE call_number IS NOT NULL AND cache_write_tokens > 0;
";
