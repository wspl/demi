//! From 0.1.17's control schema: a device records whether the backend's
//! shutdown ended its last connection, which no device migrated from 0.1.17
//! did.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_17);

const CONTROL_FROM_0_1_17: &str = "
ALTER TABLE devices ADD COLUMN ended_by_shutdown INTEGER NOT NULL DEFAULT 0 CHECK (ended_by_shutdown IN (0, 1));
";
