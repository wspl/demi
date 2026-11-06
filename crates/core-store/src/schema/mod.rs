//! The schemas of each conversation's database, and the machinery every
//! database of Demi migrates by.
//!
//! ```text
//! schema/
//!   conversation.sql                the current schema
//!   conversation/                   its history, oldest first:
//!     mod.rs                          the list of its steps
//!     from_<release>.sql              a schema a server may hold
//!     from_<release>.rs               the step from it to the next
//! ```
//!
//! A new database takes its current schema in one transaction, and records
//! the digest of the schema's text as its version. An earlier one finds its
//! version in the history and takes every step from there, in one
//! transaction. Any edit of a schema's text, a comment included, is another
//! version, so the text of a schema that a release shipped is never touched.
//!
//! A step stands alone: it uses `rusqlite`, `serde_json` and what its
//! schema hands every step, such as the blobs a conversation's values name
//! ([`ReadBlob`]). It never uses a type or a function the rest of Demi
//! serves with, which change after it: a step that needs a stored value's
//! shape keeps a copy of that shape as it was when the step was written, or
//! works on the JSON. What several steps of one history use is in a module
//! of that history beside them, which serves only them.
//!
//! The test of the crate (`tests/core-store/`) takes every step of each
//! history from an empty database of its schema and compares the result
//! with a new database of the current one. A step written in Rust has a test
//! there of its own: rows of its schema in, the rows it must leave out.
//!
//! The columns follow these conventions:
//!
//! - Times are integer milliseconds since the Unix epoch.
//! - A closed set is text a CHECK limits.
//! - JSON columns are text their reader decodes and validates.
//! - Sealed values are BLOBs.

use std::path::Path;

use rusqlite::{Connection, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};

use bytes::Bytes;
use demi_shared_types::BlobRef;

use super::StorageError;

mod conversation;
mod core;

/// A database's schema: its SQL, whose digest names it, and the schemas a
/// server may hold before it, oldest first. Each step of its history is
/// handed a `C`, what that kind of database's migrations work with.
pub struct Schema<C: ?Sized + 'static> {
    pub sql: &'static str,
    /// What a file that holds no database yet takes before its journal
    /// mode is set ([`Schema::prepare_new`]): pragmas that only take effect
    /// on an empty file.
    pub initial: &'static str,
    pub history: &'static [Shipped<C>],
}

/// A schema a server may hold, one that `main` held before a change to it,
/// and the migration from it to the schema after it in the history, or to the
/// current one after the last.
pub struct Shipped<C: ?Sized + 'static> {
    pub sql: &'static str,
    pub migration: Migration<C>,
}

/// What turns a database of one schema into one of the next.
pub enum Migration<C: ?Sized + 'static> {
    Sql(&'static str),
    /// A change SQL cannot express, such as re-encoding a stored value,
    /// with what the kind of database hands its steps.
    Code(fn(&Transaction<'_>, &C) -> rusqlite::Result<()>),
}

/// How a migration reads the bytes of a blob a database's values name, in
/// the namespace they are in; none when the namespace does not hold it.
/// It runs on the database's thread, which waits for it.
pub trait ReadBlob {
    fn read(&self, blob: &BlobRef) -> Result<Option<Bytes>, String>;
}

/// A database whose values name no blob.
pub struct NoBlobs;

impl ReadBlob for NoBlobs {
    fn read(&self, blob: &BlobRef) -> Result<Option<Bytes>, String> {
        Err(format!("this database names no blob, yet a migration read {blob}"))
    }
}

/// A core's own database's schema: the records of the principal it serves,
/// its conversations' index among them ([`crate::store::CoreStore`]).
pub const CORE: Schema<()> = Schema {
    initial: "",
    sql: include_str!("core.sql"),
    history: core::HISTORY,
};

/// Each conversation's database's schema: its nodes, the records of its
/// rows and the contents they name, its paths, commands, attachments and
/// shares.
///
/// Its history holds the forms development databases keep of it: the one
/// each content was kept once in, before a command's row kept its media,
/// and the one whose child's directory a move of the conversation overrode.
pub const CONVERSATION: Schema<dyn ReadBlob> = Schema {
    initial: "PRAGMA auto_vacuum = INCREMENTAL;",
    sql: include_str!("conversation.sql"),
    history: conversation::HISTORY,
};

/// Turns a stored error into the migration's failure, which leaves the
/// database as it was.
pub fn corrupt(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(error))
}

/// Stored data that is not what its format says, in words.
pub fn corrupt_text(message: &str) -> rusqlite::Error {
    corrupt(std::io::Error::other(message.to_owned()))
}

/// What this build does with a database as it opens it
/// (`crates/server/src/main.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaFit {
    /// It records this build's schema and opens as it is.
    Current,
    /// It holds nothing yet or records a schema in the history: opening
    /// gives it this build's schema.
    Migrates,
    /// It records another schema, as a newer release or a development build
    /// of another unpublished schema made it: opening refuses it.
    Other,
}

/// What this build would do with the database at `path` as it opens it. The
/// database is read, never written.
pub fn schema_fit<C: ?Sized>(path: &Path, schema: &Schema<C>) -> Result<SchemaFit, StorageError> {
    let connection = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let recorded: i32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    // A database that records no version and holds no table is new, and
    // `apply` gives it the schema.
    if recorded == 0 && tables(&connection)? == 0 {
        return Ok(SchemaFit::Migrates);
    }
    Ok(match schema.current(recorded) {
        Some(true) => SchemaFit::Current,
        Some(false) => SchemaFit::Migrates,
        None => SchemaFit::Other,
    })
}

/// How many tables the database of `connection` holds.
fn tables(connection: &Connection) -> rusqlite::Result<i64> {
    connection.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table'",
        [],
        |row| row.get(0),
    )
}

/// The version a database of the schema `sql` records in `user_version`:
/// the first 31 bits of its SHA-256, never 0, which SQLite gives a database
/// that records none. Any edit of the SQL is another version.
pub fn version_of(sql: &str) -> i32 {
    let digest = Sha256::digest(sql.as_bytes());
    let bits = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) >> 1;
    let version = i32::try_from(bits).expect("31 bits fit");
    version.max(1)
}

impl<C: ?Sized> Schema<C> {
    pub fn version(&self) -> i32 {
        version_of(self.sql)
    }

    /// The text of each schema in the history, oldest first.
    pub fn history_sql(&self) -> impl Iterator<Item = &'static str> {
        self.history.iter().map(|shipped| shipped.sql)
    }

    /// The migrations that bring a database of `recorded`'s schema to this
    /// one, in order; none for a version the history does not hold.
    fn migrations(&self, recorded: i32) -> Option<&'static [Shipped<C>]> {
        let from = self
            .history
            .iter()
            .position(|shipped| version_of(shipped.sql) == recorded)?;
        Some(&self.history[from..])
    }

    /// Gives a file that holds no database yet what it takes before its
    /// journal mode is set: the pragmas of [`Schema::initial`].
    pub fn prepare_new(&self, connection: &Connection) -> Result<(), StorageError> {
        let pages: i64 = connection.pragma_query_value(None, "page_count", |row| row.get(0))?;
        if pages == 0 {
            connection.execute_batch(self.initial)?;
        }
        Ok(())
    }

    /// Gives `connection`'s database this schema, migrating one of an earlier
    /// release's, with `context` handed to each step.
    pub fn apply(&self, connection: &mut Connection, path: &Path, context: &C) -> Result<(), StorageError> {
        self.migrate(connection, path, context)
    }

    fn migrate(&self, connection: &mut Connection, path: &Path, context: &C) -> Result<(), StorageError> {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let recorded: i32 =
            transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if recorded == self.version() {
            return Ok(());
        }
        if recorded == 0 && tables(&transaction)? == 0 {
            transaction.execute_batch(self.sql)?;
            transaction.pragma_update(None, "user_version", self.version())?;
            transaction.commit()?;
            return Ok(());
        }
        let migrations = self
            .migrations(recorded)
            .ok_or_else(|| StorageError::OtherSchema {
                path: path.to_owned(),
            })?;
        // A migration runs once per database, on its first open or read
        // after an upgrade, and that open or read waits for it: the log says
        // when it ran and what it cost.
        let (from, to) = (recorded, self.version());
        let database = path.file_name().map(|name| name.to_string_lossy().into_owned());
        tracing::info!(event = "schema.migration_started", database, from, to, steps = migrations.len(), "a database's schema migration starts");
        let started = std::time::Instant::now();
        let migrated = migrations
            .iter()
            .try_for_each(|shipped| {
                match shipped.migration {
                    Migration::Sql(sql) => transaction.execute_batch(sql),
                    Migration::Code(migrate) => migrate(&transaction, context),
                }
                .map_err(|source| StorageError::Migration {
                    path: path.to_owned(),
                    from: version_of(shipped.sql),
                    source,
                })
            })
            .and_then(|()| Ok(transaction.pragma_update(None, "user_version", to)?))
            .and_then(|()| Ok(transaction.commit()?));
        match &migrated {
            Ok(()) => tracing::info!(
                event = "schema.migration_ended",
                database,
                from,
                to,
                ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                "a database's schema migration ended"
            ),
            Err(error) => tracing::warn!(
                event = "schema.migration_failed",
                database,
                from,
                to,
                ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                error = error as &dyn std::error::Error,
                "a database's schema migration failed and changed nothing"
            ),
        }
        migrated
    }

    /// Whether a database that records `recorded` opens as it is, or must be
    /// migrated by `apply` first; `None` for one this build cannot open.
    pub fn current(&self, recorded: i32) -> Option<bool> {
        if recorded == self.version() {
            return Some(true);
        }
        self.migrations(recorded).map(|_| false)
    }
}

/// How many bytes of `path`'s database of `schema` an open would migrate:
/// its file's size when it records an earlier schema or none yet but holds
/// tables, else 0.
pub fn due_bytes<C: ?Sized>(path: &Path, schema: &Schema<C>) -> Result<u64, StorageError> {
    let size = match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        Ok(_) => return Ok(0),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    if size == 0 {
        return Ok(0);
    }
    Ok(match schema_fit(path, schema)? {
        SchemaFit::Migrates => size,
        SchemaFit::Current | SchemaFit::Other => 0,
    })
}
