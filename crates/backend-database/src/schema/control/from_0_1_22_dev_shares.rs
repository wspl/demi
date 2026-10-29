//! From the control schema that keeps each share's record to the one in
//! which a paired device keeps the number of its mark. A device of the earlier
//! schema takes its number at its runner's next hello.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_SHARES);

const CONTROL_FROM_SHARES: &str = "
ALTER TABLE devices ADD COLUMN mark_number INTEGER CHECK (mark_number > 0);
";
