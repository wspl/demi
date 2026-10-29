//! From the control schema with the subagent limits: Detect Title is gone, and
//! with it the counts of the messages the user sent and the last generated
//! title had seen, which only it read.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_SUBAGENT_LIMITS);

const CONTROL_FROM_SUBAGENT_LIMITS: &str = "
ALTER TABLE conversations DROP COLUMN user_messages;
ALTER TABLE conversations DROP COLUMN titled_messages;
";
