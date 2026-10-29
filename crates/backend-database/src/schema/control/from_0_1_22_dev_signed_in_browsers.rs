//! From the control schema with the signed-in browsers to the one with the
//! messages saved for later: a new table, empty, and each draft's row
//! gains the revision of the list, 0 while nothing was saved. The drafts'
//! table is rebuilt, since SQLite adds a column only after its last.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_SIGNED_IN_BROWSERS);

const CONTROL_FROM_SIGNED_IN_BROWSERS: &str = "
CREATE TABLE conversation_drafts_next (
  conversation_id   TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  revision          INTEGER NOT NULL CHECK (revision >= 1),
  document          TEXT NOT NULL,
  written           INTEGER NOT NULL CHECK (written BETWEEN 1 AND revision),
  replaced_revision INTEGER,
  replaced          TEXT,
  saved_revision    INTEGER NOT NULL CHECK (saved_revision >= 0),
  updated_at        INTEGER NOT NULL,
  CHECK ((replaced IS NULL) = (replaced_revision IS NULL))
) STRICT;
INSERT INTO conversation_drafts_next
  (conversation_id, revision, document, written, replaced_revision, replaced, saved_revision, updated_at)
  SELECT conversation_id, revision, document, written, replaced_revision, replaced, 0, updated_at
  FROM conversation_drafts;
DROP TABLE conversation_drafts;
ALTER TABLE conversation_drafts_next RENAME TO conversation_drafts;
CREATE TABLE saved_messages (
  id              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  saved_at        INTEGER NOT NULL,
  document        TEXT NOT NULL
) STRICT;
CREATE INDEX saved_messages_of_conversation ON saved_messages (conversation_id, saved_at);
";
