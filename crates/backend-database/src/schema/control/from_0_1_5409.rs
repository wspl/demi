//! From 0.1.5409's control schema, a development build's that a server
//! keeps data of, to 0.1.5469's: a project's stored name goes, since its
//! directory names it; every project keeps its saved messages per
//! conversation, as all did; and a saved message may be a project's, so the
//! table is rebuilt with every message still its conversation's.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5409);

const CONTROL_FROM_0_1_5409: &str = "
ALTER TABLE workspaces DROP COLUMN name;
ALTER TABLE workspaces ADD COLUMN saved_scope TEXT NOT NULL DEFAULT 'conversation'
  CHECK (saved_scope IN ('conversation', 'project'));
ALTER TABLE workspaces ADD COLUMN deleted_at INTEGER;
CREATE TABLE saved_messages_next (
  id              TEXT PRIMARY KEY,
  conversation_id TEXT COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  workspace_id    TEXT REFERENCES workspaces (id) ON DELETE CASCADE,
  saved_at        INTEGER NOT NULL,
  document        TEXT NOT NULL,
  CHECK ((conversation_id IS NULL) != (workspace_id IS NULL))
) STRICT;
INSERT INTO saved_messages_next (id, conversation_id, workspace_id, saved_at, document)
  SELECT id, conversation_id, NULL, saved_at, document FROM saved_messages;
DROP TABLE saved_messages;
ALTER TABLE saved_messages_next RENAME TO saved_messages;
CREATE INDEX saved_messages_of_conversation ON saved_messages (conversation_id, saved_at);
CREATE INDEX saved_messages_of_workspace ON saved_messages (workspace_id, saved_at);
";
