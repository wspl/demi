//! The server's databases: what a Demi server keeps above its users' cores.
//!
//! The crate holds:
//!
//! - the control service and its records: accounts, browser sessions,
//!   passkeys, push subscriptions, the server's settings, shares, managed
//!   operations, provider share keys, and the rows that route a request to
//!   the user whose core holds what it names before anyone signed in, a
//!   runner's device and a schedule's URL (`owners.rs`);
//! - [`core_stores::CoreStores`], which opens each user's core store in the
//!   user's directory;
//! - the control schema with its migrations;
//! - the log of the backend, the runners and the pages, with its database,
//!   its writer thread and the `tracing` layer that feeds it (`server_log`).
//!
//! What a core keeps, and the rules every schema migrates by, are
//! `core-store`'s. The object store is `core-blobs`'.
//!
//! ```text
//! data directory (DEMI_BACKEND_DATA)
//! +-- control.sqlite               the server's own records
//! +-- users/<userId>/              the user's core (`core-store`)
//! +-- log.sqlite                   the log of backend, runners and pages
//! +-- blobs/<userId>/<sha256>      user-owned bytes        (`core-blobs`)
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
//!   directory. The user's core stores the Cloud's device record and its
//!   reset intent only.
//! - A project's files belong to its Host, reached through the conversation's
//!   host access, never kept in a database.
//!
//! # Boundaries
//!
//! A product module calls the control service's operations and issues no
//! control SQL of its own. No record here holds what a core holds: a row that
//! names a user's device, schedule or conversation only routes to the core.
//!
//! Nothing here is a transaction across the control database and a core. A
//! routing row is written before the record it routes to and removed after
//! it, so a failure between them leaves at worst a row that routes to a core
//! that finds nothing, never a record no request reaches.
//!
//! # Migrations
//!
//! The control database migrates as the backend starts, before it serves;
//! each user's core store then, and a conversation's database when the
//! conversation is next opened. `demi-server` copies the databases that will
//! migrate before the new release first opens them.

pub mod accounts;
pub mod control;
pub mod core_stores;
pub mod owners;
pub mod provider_keys;
pub mod managed;
pub mod passkeys;
pub mod push;
pub mod schema;
pub use schema::{DatabaseKind, database_fit};

use demi_core_store::{StorageError, columns, sqlite};
#[cfg(feature = "testing")]
pub mod testing;
pub mod server_log;
pub mod shares;
pub mod update;
pub mod web_sessions;

