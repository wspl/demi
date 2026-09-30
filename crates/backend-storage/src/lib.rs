//! The SQLite databases (`storage.md`): the control service and every
//! control record, the conversation index, each conversation's database with
//! the tree store over it, its `blob_refs` index and its records of commands'
//! outputs, the schemas and their migrations, and the encodings of stored
//! values; and the record types it stores, which the domains above use. The
//! object store is `backend-objects`'.

pub mod accounts;
pub mod attachments;
pub mod blob_refs;
pub mod columns;
pub mod command_outputs;
pub mod control;
pub mod conversation_index;
pub mod conversations;
pub mod devices;
pub mod drafts;
pub mod exposes;
pub mod forks;
pub mod managed;
pub mod panels;
pub mod providers;
mod schema;
pub mod sequences;
pub mod sidebar;
mod sqlite;
pub mod tree;
pub mod usage;
pub mod workspaces;

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
    #[error("the file system failed: {0}")]
    Io(#[from] std::io::Error),
    /// Work handed to a blocking thread ended without an answer, because it
    /// panicked or the runtime is shutting down.
    #[error("blocking storage work did not finish: {0}")]
    Interrupted(#[from] tokio::task::JoinError),
}
