//! From 0.1.5842's control schema: a conversation keeps the replies its
//! latest answer offers, none before.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5842);

const CONTROL_FROM_0_1_5842: &str = "
CREATE TABLE conversation_suggested_replies (
  conversation_id TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  block_id        TEXT NOT NULL,
  replies         TEXT NOT NULL
) STRICT;
";
