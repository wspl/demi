//! From 0.1.20's control schema: a device keeps how pages reach it, which
//! is Automatic for every device migrated from 0.1.20, as for a new one
//! (`packages/web/src/direct/index.ts`).

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_20);

const CONTROL_FROM_0_1_20: &str = "
ALTER TABLE devices ADD COLUMN route TEXT NOT NULL DEFAULT 'automatic' CHECK (route IN ('automatic', 'direct', 'server'));
";
