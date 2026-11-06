//! The SQLite databases.
//!
//! The crate holds:
//!
//! - the control service and every control record;
//! - the conversation index;
//! - each conversation's database with the tree store over it, its records of
//!   commands' outputs and of the conversation's attachments;
//! - each user's search index;
//! - the schemas with their migrations;
//! - the record types it stores, which the domains above use.
//!
//! The object store is `backend-blobs`'.
//!
//! ```text
//! data directory (DEMI_BACKEND_DATA)
//! +-- control.sqlite               deployment-wide product records
//! +-- conversations/<id>.sqlite    one agent tree per conversation
//! +-- search/<userId>.sqlite       a user's search index, derived
//! +-- log.sqlite                   the log of backend, runners and pages (`server_log`)
//! +-- blobs/<userId>/<sha256>      user-owned bytes        (`backend-blobs`)
//! +-- native/, .attributes/        published packages, object attributes
//! +-- instance-secret              seals credentials, unless configured
//! +-- snapshots/<version>/         the databases as an upgrade found them
//! ```
//!
//! `snapshots/` is `demi-server`'s: the copy a rollback restores, which the
//! backend never reads.
//!
//! # What lives elsewhere
//!
//! - A Cloud's disks belong to the machine manager, which keeps its own state
//!   directory. The backend stores the Cloud's device record and its reset
//!   intent only.
//! - A project's files belong to its Host, reached through the conversation's
//!   host access, never kept in a conversation database.
//! - Credentials are control records, never files.
//!
//! # Boundaries
//!
//! A product module calls the control service's operations and issues no control
//! SQL of its own.
//!
//! Nothing here is a transaction across the control database, a conversation
//! database and the files. An operation that spans them, such as a Fork or a
//! Cloud reset, records its intent and checks its completion instead.
//!
//! The log of the backend, the runners and the pages, with its database, its
//! writer thread and the `tracing` layer that feeds it, lives here too. The crate holds no domain's policy: the
//! domains above it decide and it stores.
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
//! For example, 0.2.0 adds a column to the devices table:
//!
//! 1. The control schema's text changes, so its version does too.
//! 2. The history gains the schema 0.1.0 shipped with a migration that adds the
//!    column.
//! 3. A 0.2.0 backend that opens 0.1.0's control database finds that version in
//!    the history and applies the migration.
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
//! - The control database is migrated as the backend starts, before it serves; a
//!   conversation's database when the conversation is next opened, so a start
//!   does not wait for every conversation a server ever had. `demi-server`
//!   copies the databases that will migrate before the new release first opens
//!   them.

pub mod accounts;
pub mod attachments;
pub mod schedules;
pub mod control;
pub mod credentials;
pub mod conversation_index;
pub mod deletions;
pub mod devices;
pub mod drafts;
pub mod host_workflows;
pub mod forks;
pub mod instructions;
pub mod idle_compactions;
pub mod interrupted_turns;
pub mod managed;
pub mod media_jobs;
pub mod panels;
pub mod plugin_reports;
pub mod passkeys;
pub mod permissions;
pub mod plugin_secrets;
mod plugin_store;
pub mod plugin_values;
pub mod port_forwards;
pub mod providers;
pub mod push;
pub mod references;
pub mod running;
pub mod saved;
pub mod schema;
pub use schema::{DatabaseKind, database_fit};

use demi_core_store::{StorageError, columns, sqlite};
#[cfg(feature = "testing")]
pub mod testing;
mod selection;
pub use selection::{CLOUD_HOME, Selection, Unselectable, Worktree};
pub mod server_log;
pub mod shares;
pub mod sidebar;
pub mod subagents;
pub mod suggested_replies;
pub mod update;
pub mod usage;
pub mod web_sessions;
pub mod projects;
pub mod workspaces;

