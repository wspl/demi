//! From 0.1.5844's control schema: the server settings say whether the
//! server gets a newer release ready as soon as a check finds it, which
//! every server starts without.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5844);

const CONTROL_FROM_0_1_5844: &str = "
ALTER TABLE server_settings ADD COLUMN prepare_ahead INTEGER NOT NULL DEFAULT 0 CHECK (prepare_ahead IN (0, 1));
";
