//! From 0.1.5922's control schema, which 0.1.5924 shipped too: a turn's
//! ledger rows are found by its id, for what each turn of a page of a
//! transcript cost (`crates/backend-database/src/usage.rs` § A cost report).

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5922);

const CONTROL_FROM_0_1_5922: &str = "
CREATE INDEX usage_ledger_turn ON usage_ledger (conversation_id, turn_id)
  WHERE turn_id IS NOT NULL;
";
