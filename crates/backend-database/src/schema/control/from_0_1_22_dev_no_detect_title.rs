//! From the control schema without Detect Title: each subagent limit keeps its
//! number while it is lifted (`crates/agent-server/src/server/tree/profiles.rs`),
//! so a limit stored as Unlimited, a null, takes its default number and stays
//! Unlimited.
//!
//! The table is rebuilt, since SQLite cannot make a column NOT NULL in place.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_NO_DETECT_TITLE);

const CONTROL_FROM_NO_DETECT_TITLE: &str = "
CREATE TABLE user_subagents_kept (
  user_id             TEXT PRIMARY KEY REFERENCES users (id),
  enabled             INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  max_live            INTEGER NOT NULL CHECK (max_live >= 1),
  max_live_unlimited  INTEGER NOT NULL CHECK (max_live_unlimited IN (0, 1)),
  max_depth           INTEGER NOT NULL CHECK (max_depth >= 1),
  max_depth_unlimited INTEGER NOT NULL CHECK (max_depth_unlimited IN (0, 1))
) STRICT;
INSERT INTO user_subagents_kept
  SELECT user_id, enabled, coalesce(max_live, 10), max_live IS NULL,
         coalesce(max_depth, 1), max_depth IS NULL
  FROM user_subagents;
DROP TABLE user_subagents;
ALTER TABLE user_subagents_kept RENAME TO user_subagents;
";
