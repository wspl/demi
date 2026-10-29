//! From 0.1.11's control schema: the retention pass, which read when each
//! conversation was last live, is gone, and the column with it.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_11);

const CONTROL_FROM_0_1_11: &str = "
ALTER TABLE conversations DROP COLUMN live_at;
";
