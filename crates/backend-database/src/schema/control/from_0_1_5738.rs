//! From 0.1.5738's control schema to the current one: the rotation Balanced
//! is Smart.
//!
//! - The providers table's text names `smart` in its CHECK where it named
//!   `balanced`: a change of the text alone, as SQLite's procedure for a
//!   change of constraints gives it (lang_altertable.html § 7), which keeps
//!   the rows and the accounts' foreign keys to them.
//! - Each entry that rotated by Balanced rotates by Smart. The integrity
//!   check, which tests CHECK constraints, proves none is left.

use rusqlite::Transaction;

use super::super::{Migration, ReadBlob, corrupt_text};

pub(super) const MIGRATION: Migration = Migration::Code(control_from_0_1_5738);

fn control_from_0_1_5738(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    let version: i64 = transaction.pragma_query_value(None, "schema_version", |row| row.get(0))?;
    transaction.pragma_update(None, "writable_schema", true)?;
    transaction.execute(
        "UPDATE sqlite_schema SET sql = replace(sql, '''balanced''', '''smart''')
           WHERE type = 'table' AND name = 'providers'",
        [],
    )?;
    transaction.pragma_update(None, "schema_version", version + 1)?;
    transaction.pragma_update(None, "writable_schema", false)?;
    transaction.execute("UPDATE providers SET rotation = 'smart' WHERE rotation = 'balanced'", [])?;
    let problems: Vec<String> = transaction
        .prepare("PRAGMA integrity_check")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if problems != ["ok"] {
        return Err(corrupt_text(&format!("the entries' rotations fail their check: {problems:?}")));
    }
    Ok(())
}
