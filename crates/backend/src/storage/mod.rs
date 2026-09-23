//! The databases (`storage.md`).

pub(crate) mod control;
mod schema;
mod sqlite;

/// Why a storage operation failed.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("SQLite failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("the schema could not be applied: {0}")]
    Schema(#[from] rusqlite_migration::Error),
    /// Replication and concurrent readers need WAL, which the file system
    /// refused.
    #[error("the database stays in journal mode {0}, not WAL")]
    JournalMode(String),
    /// A stored value outside its type; nothing repairs it.
    #[error("{table}.{column} holds an invalid value: {reason}")]
    Corrupt {
        table: &'static str,
        column: &'static str,
        reason: String,
    },
    /// A computed time, such as an expiry, falls outside the range of times.
    #[error("a time is out of range: {0}")]
    Time(jiff::Error),
    #[error("the database is closed")]
    Closed,
}
