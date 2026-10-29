//! From 0.1.32's control schema, which 0.1.28 to 0.1.31 shipped too, to
//! 0.1.36's.
//!
//! - A usage ledger row keeps what each kind of its tokens would cost on the
//!   vendor's API and the hashes of the system prompt and the tools its request
//!   sent (`crates/backend-database/src/usage.rs`), null for every row written
//!   before.
//! - The ledger is read by conversation, for a conversation's cost (§ A
//!   conversation's cost).

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_32);

const CONTROL_FROM_0_1_32: &str = "
ALTER TABLE usage_ledger ADD COLUMN api_price_input REAL CHECK (api_price_input >= 0);
ALTER TABLE usage_ledger ADD COLUMN api_price_cache_write REAL CHECK (api_price_cache_write >= 0);
ALTER TABLE usage_ledger ADD COLUMN api_price_cache_read REAL CHECK (api_price_cache_read >= 0);
ALTER TABLE usage_ledger ADD COLUMN api_price_output REAL CHECK (api_price_output >= 0);
ALTER TABLE usage_ledger ADD COLUMN system_hash TEXT;
ALTER TABLE usage_ledger ADD COLUMN tools_hash TEXT;
CREATE INDEX usage_ledger_conversation ON usage_ledger (conversation_id);
";
