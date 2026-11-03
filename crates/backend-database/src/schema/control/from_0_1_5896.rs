//! From 0.1.5896's control schema: the server settings say which private
//! destinations Clouds reach, none on every server until its master chooses.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5896);

const CONTROL_FROM_0_1_5896: &str = "
ALTER TABLE server_settings ADD COLUMN private_network TEXT NOT NULL DEFAULT 'off' CHECK (private_network IN ('off', 'entire', 'only'));
ALTER TABLE server_settings ADD COLUMN private_destinations TEXT NOT NULL DEFAULT '[]';
";
