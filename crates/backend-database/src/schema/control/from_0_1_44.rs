//! From 0.1.44's control schema, which 0.1.39 to 0.1.43 shipped too, to
//! 0.1.48's.
//!
//! - A move applies at once, so no conversation keeps a move that waits for
//!   its work to end; one an agent asked for before the upgrade is not made.
//!   Each column goes on its own, the one whose CHECK names the others first.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_44);

const CONTROL_FROM_0_1_44: &str = "
ALTER TABLE conversations DROP COLUMN pending_workspace_id;
ALTER TABLE conversations DROP COLUMN pending_kind;
ALTER TABLE conversations DROP COLUMN pending_id;
ALTER TABLE conversations DROP COLUMN pending_device_id;
ALTER TABLE conversations DROP COLUMN pending_path;
";
