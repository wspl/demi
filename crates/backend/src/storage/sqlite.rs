//! The settings every database connection opens with (`storage.md`), and the
//! errors of a connection's own thread.

use std::time::Duration;

use rusqlite::Connection;

use super::StorageError;

/// How long a statement waits for a lock another connection holds.
pub(super) const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// WAL, foreign keys and the busy timeout.
pub(super) fn configure(connection: &Connection) -> Result<(), StorageError> {
    let mode: String =
        connection.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StorageError::JournalMode(mode));
    }
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    Ok(())
}

/// The error of work that ran on a connection's thread.
pub(super) fn flatten(error: tokio_rusqlite::Error<StorageError>) -> StorageError {
    match error {
        tokio_rusqlite::Error::Error(error) => error,
        tokio_rusqlite::Error::Close((_, error)) => StorageError::Sqlite(error),
        _ => StorageError::Closed,
    }
}

/// Closes a connection and waits for its thread to close the database.
pub(super) async fn close(connection: tokio_rusqlite::Connection) -> Result<(), StorageError> {
    match connection.close().await {
        Ok(()) => Ok(()),
        Err(tokio_rusqlite::Error::Close((_, error)) | tokio_rusqlite::Error::Error(error)) => {
            Err(StorageError::Sqlite(error))
        }
        Err(_) => Err(StorageError::Closed),
    }
}
