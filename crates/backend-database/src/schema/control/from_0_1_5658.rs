//! From 0.1.5658's control schema to 0.1.5672's: a plugin's Host
//! directory may be one workspace's, and every directory kept is every job's.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5658);

const CONTROL_FROM_0_1_5658: &str = "
ALTER TABLE plugin_directories ADD COLUMN workspace_id TEXT REFERENCES workspaces (id);
";
