//! Gives conversations' browsers their view links, none yet.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_NO_BROWSER_VIEWS);

const CONTROL_FROM_NO_BROWSER_VIEWS: &str = r"
CREATE TABLE browser_views (
  token_hash      TEXT PRIMARY KEY,
  id              TEXT NOT NULL UNIQUE,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  tab             TEXT NOT NULL,
  read_only       INTEGER NOT NULL CHECK (read_only IN (0, 1)),
  created_at      INTEGER NOT NULL,
  expires_at      INTEGER NOT NULL,
  CHECK (expires_at > created_at)
) STRICT;
CREATE INDEX browser_views_conversation ON browser_views (conversation_id, expires_at);
";
