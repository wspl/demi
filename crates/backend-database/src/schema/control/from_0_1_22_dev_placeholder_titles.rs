//! From the control schema where a placeholder title holds no words to the
//! one that keeps the root turns a graceful shutdown interrupted. A database
//! a shutdown of the earlier release closed marks none: its interrupted
//! turns wait for the user's Resume.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_PLACEHOLDER_TITLES);

const CONTROL_FROM_PLACEHOLDER_TITLES: &str = "
CREATE TABLE interrupted_turns (
  conversation_id TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE
) STRICT;
";
