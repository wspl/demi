//! From 0.1.5586's control schema to 0.1.5614's: each turn call's cache
//! miss is judged again by what it read of the cache (`ledger.rs`); one
//! that only added to its prompt missed nothing.

use rusqlite::Transaction;

use super::super::{Migration, ReadBlob};

pub(super) const MIGRATION: Migration = Migration::Code(control_from_0_1_5586);

fn control_from_0_1_5586(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    super::ledger::judge_misses(transaction)
}
