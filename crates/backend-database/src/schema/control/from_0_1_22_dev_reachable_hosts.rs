//! From the control schema in which a conversation keeps the hosts it may
//! reach to the one that keeps each share's record. A database of the earlier
//! schema has shared nothing.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_REACHABLE_HOSTS);

const CONTROL_FROM_REACHABLE_HOSTS: &str = "
CREATE TABLE shares (
  token           TEXT PRIMARY KEY,
  user_id         TEXT NOT NULL REFERENCES users (id),
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  kind            TEXT NOT NULL CHECK (kind IN ('conversation', 'answer')),
  block_id        TEXT,
  title           TEXT,
  taken_at        INTEGER NOT NULL,
  CHECK ((kind = 'answer') = (block_id IS NOT NULL))
) STRICT;
CREATE UNIQUE INDEX shares_of_conversation ON shares (conversation_id) WHERE kind = 'conversation';
CREATE UNIQUE INDEX shares_of_answer ON shares (conversation_id, block_id) WHERE kind = 'answer';
";
