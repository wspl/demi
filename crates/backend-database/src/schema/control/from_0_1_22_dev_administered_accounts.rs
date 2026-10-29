//! From the control schema with administered accounts to the one with a
//! conversation's Workflow menu choice
//! (`crates/backend-user-shard/src/conversation/workflows.rs`): every
//! conversation uses Global Workflow, as a new one does.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_ADMINISTERED_ACCOUNTS);

const CONTROL_FROM_ADMINISTERED_ACCOUNTS: &str = "
ALTER TABLE conversations ADD COLUMN workflow_kind TEXT NOT NULL DEFAULT 'default' CHECK (workflow_kind IN ('default', 'none', 'workflow'));
ALTER TABLE conversations ADD COLUMN workflow_id TEXT CHECK ((workflow_kind = 'workflow') = (workflow_id IS NOT NULL));
";
