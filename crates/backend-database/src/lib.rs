//! The SQLite databases: the control service and every control record, the
//! conversation index, each conversation's database with the tree store over
//! it, its records of commands' outputs and of the conversation's
//! attachments, each user's search index and the schemas with their
//! migrations; and the record types it stores, which the domains above use. The object store is `backend-blobs`'.
//!
//! ```text
//! data directory (DEMI_BACKEND_DATA)
//! +-- control.sqlite               deployment-wide product records
//! +-- conversations/<id>.sqlite    one agent tree per conversation
//! +-- search/<userId>.sqlite       a user's search index, derived
//! +-- log.sqlite                   the server log (`server_log`)
//! +-- blobs/<userId>/<sha256>      user-owned bytes        (`backend-blobs`)
//! +-- native/, .attributes/        published packages, object attributes
//! +-- instance-secret              seals credentials, unless configured
//! +-- snapshots/<version>/         the databases as an upgrade found them
//! ```
//!
//! `snapshots/` is `demi-server`'s: the copy a rollback restores, which the
//! backend never reads. What lives elsewhere stays there: a Cloud's disks
//! belong to the machine manager, which keeps its own state directory (the
//! backend stores the Cloud's device record and its reset intent only), and a
//! project's files belong to its Host, reached through the conversation's
//! host access, never kept in a conversation database. Credentials are
//! control records, never files.
//!
//! A product module calls the control service's operations and issues no
//! control SQL of its own. Nothing here is a transaction across the control
//! database, a conversation database and the files: an operation that spans
//! them, such as a Fork or a Cloud reset, records its intent and checks its
//! completion instead.
//!
//! # Schemas and migrations
//!
//! Each kind of database has a schema, one SQL text, and the history of the
//! schemas that published releases shipped before it, each with the migration
//! that leads from it to the next. A schema's version is a digest of its
//! text, the first 31 bits of its SHA-256, which the database records in
//! SQLite's `user_version`: any edit of the text, a comment included, is
//! another version, so the text of a schema that a release shipped is never
//! touched. For example, 0.2.0 adds a column to the devices table. The
//! control schema's text changes, so its version does too, and the history
//! gains the schema 0.1.0 shipped with a migration that adds the column. A
//! 0.2.0 backend that opens 0.1.0's control database finds that version in
//! the history and applies the migration.
//!
//! - The first change to a schema after a release or a deploy adds the schema
//!   that release or deploy shipped to the history, with its migration. A
//!   later change before the next release or deploy extends that migration,
//!   so two shipments apart are one step whatever changed between them. A
//!   database that records a version outside the history, such as one a
//!   development build of another unpublished schema made, is refused, never
//!   changed.
//! - Every published release's schema is in the history, so migrations are
//!   written and tested on real upgrades long before a user depends on them.
//!   The first formal release clears the history; its schemas start a new
//!   one.
//! - A migration is SQL, or Rust where SQL cannot express the change, such as
//!   re-encoding a stored value; what it writes goes through the encoding and
//!   validation of any write.
//! - A value stored as JSON is part of its schema: the schema's text names
//!   the format of each such column in a comment (`-- blocks.block:
//!   transcript block format 2`). A release that changes what the value
//!   holds raises that number, so the text and the version change and the
//!   migration re-encodes the stored values. Decoding stays strict: a value
//!   of an earlier format is never read as the current one.
//! - The control database is migrated as the backend starts, before it
//!   serves; a conversation's database when the conversation is next opened,
//!   so a start does not wait for every conversation a server ever had.
//!   `demi-server` copies the databases that will migrate before the new
//!   release first opens them.
//!
//! The server log, with its database, its writer thread and the `tracing` layer
//! that feeds it, lives here too. The crate holds no domain's policy: the
//! domains above it decide and it stores.

pub mod accounts;
pub mod attachments;
pub mod automations;
pub mod columns;
pub mod command_outputs;
pub mod conversation_attachments;
pub mod control;
pub mod credentials;
pub mod conversation_index;
pub mod conversations;
pub mod deletions;
pub mod devices;
pub mod drafts;
pub mod exposed_ports;
pub mod forks;
pub mod instructions;
pub mod interrupted_turns;
pub mod managed;
pub mod panels;
pub mod passkeys;
mod path;
pub mod permissions;
pub mod plugin_secrets;
pub mod plugin_values;
pub mod port_forwards;
pub mod providers;
pub mod push;
pub mod references;
pub mod running;
pub mod saved;
mod schema;
#[cfg(feature = "testing")]
pub use schema::testing as schema_testing;
pub use schema::{DatabaseKind, SchemaFit, schema_fit};
mod selection;
pub use selection::{CLOUD_HOME, Selection, Unselectable};
pub mod search;
pub mod server_log;
pub mod shares;
pub mod sequences;
pub mod side_questions;
pub mod sidebar;
mod sqlite;
pub mod subagents;
pub mod tree;
pub mod usage;
pub mod web_sessions;
pub mod workspaces;

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
