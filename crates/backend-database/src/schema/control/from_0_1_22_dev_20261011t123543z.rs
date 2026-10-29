//! From the control schema of the development build 0.1.22-dev.20261011T123543Z,
//! which a server keeps data of: each user's subagent settings hold the two limits
//! (`crates/agent-server/src/server/tree/profiles.rs`).
//!
//! A user who turned the Subagents switch has them at their defaults, as a user
//! with no row has.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_22_DEV_20261011T123543Z);

const CONTROL_FROM_0_1_22_DEV_20261011T123543Z: &str = "
ALTER TABLE user_subagents ADD COLUMN max_live INTEGER DEFAULT 10 CHECK (max_live >= 1);
ALTER TABLE user_subagents ADD COLUMN max_depth INTEGER DEFAULT 1 CHECK (max_depth >= 1);
";
