//! The databases and the object store (`storage.md`).

pub(crate) mod attachments;
pub(crate) mod blobs;
pub(crate) mod changes;
pub(crate) mod columns;
pub(crate) mod control;
pub(crate) mod conversation_index;
pub(crate) mod conversations;
pub(crate) mod devices;
pub(crate) mod forks;
pub(crate) mod managed;
pub(crate) mod objects;
pub(crate) mod panels;
pub(crate) mod providers;
mod schema;
pub(crate) mod sidebar;
mod sqlite;
pub(crate) mod tree;
pub(crate) mod usage;
pub(crate) mod workspaces;

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
    #[error("the object store failed: {0}")]
    Objects(#[from] object_store::Error),
    #[error("the file system failed: {0}")]
    Io(#[from] std::io::Error),
    /// Work handed to a blocking thread ended without an answer, because it
    /// panicked or the runtime is shutting down.
    #[error("blocking storage work did not finish: {0}")]
    Interrupted(#[from] tokio::task::JoinError),
}
