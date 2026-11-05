//! Turns the view links of conversations' browsers into view links of a tab
//! or of a whole conversation, either of which may never expire.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_BROWSER_VIEWS);

const CONTROL_FROM_BROWSER_VIEWS: &str = r"
CREATE TABLE view_links (
  token_hash      TEXT PRIMARY KEY,
  id              TEXT NOT NULL UNIQUE,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  tab             TEXT,
  read_only       INTEGER NOT NULL CHECK (read_only IN (0, 1)),
  created_at      INTEGER NOT NULL,
  expires_at      INTEGER,
  CHECK (expires_at IS NULL OR expires_at > created_at),
  CHECK (tab IS NOT NULL OR read_only = 1)
) STRICT;
INSERT INTO view_links (token_hash, id, conversation_id, tab, read_only, created_at, expires_at)
  SELECT token_hash, id, conversation_id, tab, read_only, created_at, expires_at FROM browser_views;
DROP TABLE browser_views;
CREATE INDEX view_links_conversation ON view_links (conversation_id, expires_at);
";
