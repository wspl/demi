//! From the control schema in which an attached host goes by its device's
//! name to the one in which a subagent profile has a Context of its own
//! (`crates/agent-server/src/server/tree/profiles.rs`): every stored profile follows the user's
//! limit, as it did.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_DEVICE_NAMES);

const CONTROL_FROM_DEVICE_NAMES: &str =
    "ALTER TABLE subagent_profiles ADD COLUMN context_limit INTEGER CHECK (context_limit >= 1);";
