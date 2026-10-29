//! From 0.1.15's control schema: a conversation counts the changes of its
//! attached hosts, from 0.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_15);

const CONTROL_FROM_0_1_15: &str = "
ALTER TABLE conversations ADD COLUMN hosts_revision INTEGER NOT NULL DEFAULT 0 CHECK (hosts_revision >= 0);
";
