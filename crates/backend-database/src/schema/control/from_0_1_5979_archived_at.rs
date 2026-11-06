//! From the control schema whose conversation records when it was archived:
//! a suggested replies answer is a row that stays, with what answered and what
//! the user did with it. The conversation offers its newest row. No release
//! recorded when a kept row was answered, so one takes the conversation's last
//! activity, and its source stays unknown.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5979_ARCHIVED_AT);

const CONTROL_FROM_0_1_5979_ARCHIVED_AT: &str = "
CREATE TABLE conversation_suggested_replies_next (
  id              INTEGER PRIMARY KEY,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  block_id        TEXT NOT NULL,
  answered_at     INTEGER NOT NULL,
  provider_id     TEXT,
  model           TEXT,
  instruction_hash TEXT,
  response        TEXT,
  replies         TEXT NOT NULL,
  used_reply      INTEGER,
  used_as         TEXT CHECK (used_as IN ('sent', 'edited')),
  used_at         INTEGER
) STRICT;
INSERT INTO conversation_suggested_replies_next (conversation_id, block_id, answered_at, replies)
  SELECT kept.conversation_id, kept.block_id, conversations.updated_at, kept.replies
  FROM conversation_suggested_replies AS kept
  JOIN conversations ON conversations.id = kept.conversation_id;
DROP TABLE conversation_suggested_replies;
ALTER TABLE conversation_suggested_replies_next RENAME TO conversation_suggested_replies;
CREATE INDEX conversation_suggested_replies_latest ON conversation_suggested_replies (conversation_id, id);
";
