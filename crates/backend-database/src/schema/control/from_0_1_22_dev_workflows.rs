//! From the control schema with a conversation's workflow to the one that
//! keeps the edits of messages saved for later and their sends for a day:
//! two new tables, empty.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_WORKFLOWS);

const CONTROL_FROM_WORKFLOWS: &str = "
-- An edit of a message saved for later, by the page's id for it, with its
-- answer, kept a day so the same edit sent again answers as it first did.
-- answer: the edit's answer, `{ draft, revisions }` as `web-api.md`
-- § Messages saved for later names it, as JSON.
CREATE TABLE saved_edits (
  op              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  answer          TEXT NOT NULL,
  made_at         INTEGER NOT NULL
) STRICT;
CREATE INDEX saved_edits_by_age ON saved_edits (made_at);

-- A message saved for later that a send took, with the turn the send became,
-- kept a day so the same send, or a save of it, sent again is known.
-- document: its text and files as saved_messages keeps them.
CREATE TABLE saved_sends (
  id              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  turn_id         TEXT NOT NULL,
  saved_at        INTEGER NOT NULL,
  document        TEXT NOT NULL,
  sent_at         INTEGER NOT NULL
) STRICT;
CREATE INDEX saved_sends_by_age ON saved_sends (sent_at);
";
