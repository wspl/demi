//! From the control schema of the media jobs, which no release shipped: any
//! entry whose family connects through devices may name one, a subscription's
//! as well as an API key's.
//!
//! The providers table's text drops the CHECK that limited the column to
//! API-key entries: a change of the text alone, as SQLite's procedure for a
//! change of constraints gives it (lang_altertable.html § 7), which keeps the
//! rows and the accounts' foreign keys to them. Which family connects through
//! a device is the families' to say, not the table's.

use rusqlite::Transaction;

use super::super::{Migration, ResealSealed};

pub(super) const MIGRATION: Migration = Migration::Code(control_from_0_1_5979_media_jobs);

fn control_from_0_1_5979_media_jobs(transaction: &Transaction<'_>, _: &dyn ResealSealed) -> rusqlite::Result<()> {
    let version: i64 = transaction.pragma_query_value(None, "schema_version", |row| row.get(0))?;
    transaction.pragma_update(None, "writable_schema", true)?;
    transaction.execute(
        "UPDATE sqlite_schema
           SET sql = replace(sql, 'CHECK (through_device_id IS NULL OR credential_kind = ''api_key'')', '')
           WHERE type = 'table' AND name = 'providers'",
        [],
    )?;
    transaction.pragma_update(None, "schema_version", version + 1)?;
    transaction.pragma_update(None, "writable_schema", false)
}
