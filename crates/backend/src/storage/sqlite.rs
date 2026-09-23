//! The settings every database connection opens with (`storage.md`).

use std::time::Duration;

use rusqlite::Connection;

use super::StorageError;

/// WAL, foreign keys and a five-second busy timeout.
pub(super) fn configure(connection: &Connection) -> Result<(), StorageError> {
    let mode: String =
        connection.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StorageError::JournalMode(mode));
    }
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    Ok(())
}
