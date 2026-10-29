//! From 0.1.16's control schema: a device keeps the runner release its
//! runner reports, which a device migrated from 0.1.16 learns at its
//! runner's next hello; and a conversation's deletion is recorded as
//! pending until its last step.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_16);

const CONTROL_FROM_0_1_16: &str = "
ALTER TABLE devices ADD COLUMN runner_version TEXT;
CREATE TABLE conversation_deletions (
  id         TEXT PRIMARY KEY COLLATE NOCASE,
  user_id    TEXT NOT NULL REFERENCES users (id),
  deleted_at INTEGER NOT NULL
) STRICT;
";
