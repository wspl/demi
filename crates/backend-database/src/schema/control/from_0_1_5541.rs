//! From 0.1.5541's control schema to 0.1.5565's: an idle compaction
//! keeps the tokens its request carried, which the user's minimum context
//! is compared with. The rows stored before it qualified from 200,000
//! tokens, the minimum until then.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5541);

const CONTROL_FROM_0_1_5541: &str = "
CREATE TABLE idle_compactions_next (
  conversation_id TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  due_at          INTEGER NOT NULL,
  lifetime_ms     INTEGER NOT NULL CHECK (lifetime_ms > 0),
  carried         INTEGER NOT NULL CHECK (carried > 0)
) STRICT;
INSERT INTO idle_compactions_next (conversation_id, due_at, lifetime_ms, carried)
  SELECT conversation_id, due_at, lifetime_ms, 200000 FROM idle_compactions;
DROP TABLE idle_compactions;
ALTER TABLE idle_compactions_next RENAME TO idle_compactions;
";
