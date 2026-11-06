//! The core's storage: what one core keeps in the data directory it is
//! given (`docs/decisions` 0296's core).
//!
//! The crate holds:
//!
//! - each conversation's database with the tree store over it, its records
//!   of commands' outputs and of the conversation's attachments, and the
//!   lists of its shares;
//! - the search index;
//! - the schemas with their migrations, and the machinery every database of
//!   Demi migrates by, which the server's own databases use too.
//!
//! ```text
//! data directory
//! +-- conversations/<id>.sqlite    one agent tree per conversation
//! +-- search.sqlite                the search index, derived
//! ```
//!
//! # Schemas and migrations
//!
//! Each kind of database has a schema, one SQL text, and the history of the
//! schemas that published releases shipped before it, each with the migration
//! that leads from it to the next.
//!
//! A schema's version is a digest of its text, the first 31 bits of its SHA-256,
//! which the database records in SQLite's `user_version`. Any edit of the text, a
//! comment included, is another version, so the text of a schema that a release
//! shipped is never touched.
//!
//! For example, 0.2.0 adds a column to a conversation's nodes table:
//!
//! 1. The conversation schema's text changes, so its version does too.
//! 2. The history gains the schema 0.1.0 shipped with a migration that adds the
//!    column.
//! 3. A 0.2.0 core that opens 0.1.0's conversation database finds that version
//!    in the history and applies the migration.
//!
//! The rules:
//!
//! - Every change to a schema adds the schema `main` holds to the history, with
//!   its migration: any commit of `main` may be published as a Canary, so any
//!   schema `main` held may be on a server. A database that records a version
//!   outside the history, such as one a local development build made, is
//!   refused, never changed.
//! - Every published release's schema is in the history, so migrations are
//!   written and tested on real upgrades long before a user depends on them.
//!   The crate's test takes every step of each history to the current schema
//!   and compares the result with a new database (`schema/mod.rs`).
//! - A migration is SQL, or Rust where SQL cannot express the change, such as
//!   re-encoding a stored value. It stands alone: it uses none of the types
//!   and functions the rest of Demi serves with, which change after it
//!   (`schema/mod.rs`).
//! - A value stored as JSON is part of its schema: the schema's text names the
//!   format of each such column in a comment (`-- nodes.state: checkpoint state
//!   format 8`). A release that changes what the value holds raises that number,
//!   so the text and the version change and the migration re-encodes the stored
//!   values. Decoding stays strict: a value of an earlier format is never read as
//!   the current one.
//! - A transcript row is a record that names its contents (`records.rs`,
//!   `contents.rs`) and carries its block format, so a change of the block
//!   format rewrites no row: a release reads each format it knows. A migration
//!   adds or drops the columns queries read, and never decodes every row.
//! - A conversation's database is migrated when the conversation is next
//!   opened, so a start does not wait for every conversation a core ever had,
//!   or before the core serves when whoever starts it asks
//!   ([`conversations::ConversationStores::prepare`]).
//!
//! # Boundaries
//!
//! Nothing here is a transaction across two databases and the files. An
//! operation that spans them, such as a Fork, records its intent and checks
//! its completion instead. The crate holds no domain's policy: the domains
//! above it decide and it stores.

pub mod columns;
pub mod command_outputs;
mod contents;
pub mod conversation_attachments;
pub mod conversations;
mod path;
mod records;
pub mod running;
pub mod schema;
pub mod search;
pub mod sequences;
pub mod shares;
pub mod side_questions;
pub mod sqlite;
pub mod tree;

/// Why a storage operation failed.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("SQLite failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The database was made by a newer release or another build of Demi,
    /// whose schema this build does not read and never changes.
    #[error(
        "{} was made by a newer release or another build of Demi: return to the release that made it, or restore the databases of this release with `demi-server rollback`",
        path.display()
    )]
    OtherSchema { path: std::path::PathBuf },
    /// A migration failed; its transaction left the database as it was.
    #[error(
        "{} could not be migrated from schema version {from}: {source}",
        path.display()
    )]
    Migration {
        path: std::path::PathBuf,
        from: i32,
        #[source]
        source: rusqlite::Error,
    },
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

/// The store's own failure, which the plugin host passed on, or corrupt data
/// the host found: a sealed secret that does not open, the one failure it
/// raises itself.
impl From<demi_plugin_host::store::StoreError> for StorageError {
    fn from(error: demi_plugin_host::store::StoreError) -> Self {
        error.downcast::<Self>().unwrap_or_else(|error| Self::Corrupt {
            table: "plugin_secrets",
            column: "sealed",
            reason: error.to_string(),
        })
    }
}
