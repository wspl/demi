//! From 0.1.48's control schema, which 0.1.46 and 0.1.47 shipped too, to the
//! next one.
//!
//! - A conversation's ledger rows are found by their end, for the cost of its
//!   last hour and day (`crates/backend-database/src/usage.rs` § The last hour
//!   and the last day); the index by conversation alone, which that one serves
//!   too, goes. The ledger's later indexes are made again after it, so the
//!   database lists its indexes in the order a new one does.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_48);

const CONTROL_FROM_0_1_48: &str = "
DROP INDEX usage_ledger_conversation;
DROP INDEX usage_ledger_agent;
DROP INDEX usage_ledger_cache_writes;
CREATE INDEX usage_ledger_conversation ON usage_ledger (conversation_id, ended_at);
CREATE INDEX usage_ledger_agent ON usage_ledger (conversation_id, agent);
CREATE INDEX usage_ledger_cache_writes ON usage_ledger (conversation_id, agent, model_id)
  WHERE call_number IS NOT NULL AND cache_write_tokens > 0;
";
