//! From 0.1.13's control schema: Host expose is gone, and its records with
//! it; a device keeps the operating system its runner reports, which a
//! device migrated from 0.1.13 learns at its runner's next hello.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_13);

const CONTROL_FROM_0_1_13: &str = "
DROP TABLE exposes;
ALTER TABLE devices ADD COLUMN os TEXT;
";
