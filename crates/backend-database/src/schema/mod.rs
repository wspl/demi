//! The schemas of the control database and of each conversation's database.
//!
//! ```text
//! schema/
//!   control.sql, conversation.sql   each database's current schema
//!   control/, conversation/         its history, oldest first:
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
//! A step stands alone: it uses `rusqlite`, `serde_json` and what the
//! migration is handed, the blobs a conversation's values name
//! ([`ReadBlob`]) and the vault that seals again what it sealed
//! ([`ResealSealed`]). It never uses a type or a function the rest of Demi
//! serves with, which change after it: a step that needs a stored value's
//! shape keeps a copy of that shape as it was when the step was written, or
//! works on the JSON. What several steps of one history use is in a module
//! of that history beside them, which serves only them.
//!
//! The test of the crate (`tests/backend-database/`) takes every step of
//! each history from an empty database of its schema and compares the result
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
use super::providers::{NoSealed, ResealSealed};

mod control;
mod conversation;

/// A database's schema: its SQL, whose digest names it, and the schemas a
/// server may hold before it, oldest first.
pub(crate) struct Schema {
    sql: &'static str,
    /// What a file that holds no database yet takes before its journal
    /// mode is set ([`Schema::prepare_new`]): pragmas that only take effect
    /// on an empty file.
    initial: &'static str,
    history: &'static [Shipped],
}

/// A schema a server may hold, one that `main` held before a change to it,
/// and the migration from it to the schema after it in the history, or to the
/// current one after the last.
pub(crate) struct Shipped {
    sql: &'static str,
    migration: Migration,
}

/// What turns a database of one schema into one of the next.
pub(crate) enum Migration {
    Sql(&'static str),
    /// A change SQL cannot express, such as re-encoding a stored value,
    /// which may read the blobs the database's values name.
    Code(fn(&Transaction<'_>, &dyn ReadBlob) -> rusqlite::Result<()>),
    /// A change that re-encodes sealed values, which only their owner can
    /// open and seal again.
    Reseal(fn(&Transaction<'_>, &dyn ResealSealed) -> rusqlite::Result<()>),
}

/// How a migration reads the bytes of a blob a database's values name, in
/// the namespace they are in; none when the namespace does not hold it.
/// It runs on the database's thread, which waits for it.
pub trait ReadBlob {
    fn read(&self, blob: &BlobRef) -> Result<Option<Bytes>, String>;
}

/// A database whose values name no blob, such as the control database.
pub(crate) struct NoBlobs;

impl ReadBlob for NoBlobs {
    fn read(&self, blob: &BlobRef) -> Result<Option<Bytes>, String> {
        Err(format!("this database names no blob, yet a migration read {blob}"))
    }
}

/// The control database's schema. Its history holds every schema a server
/// may hold before the current one (`upgrades.md` § A development build on
/// a server).
pub(crate) const CONTROL: Schema = Schema {
    initial: "",
    sql: include_str!("control.sql"),
    history: control::HISTORY,
};

/// Each conversation's database's schema: its nodes, the records of its
/// rows and the contents they name, its paths, commands, attachments and
/// shares.
///
/// Its history holds the form development databases keep of it: the one
/// each content was kept once in, before a command's row kept its media.
pub(crate) const CONVERSATION: Schema = Schema {
    initial: "PRAGMA auto_vacuum = INCREMENTAL;",
    sql: include_str!("conversation.sql"),
    history: conversation::HISTORY,
};

/// Turns a stored error into the migration's failure, which leaves the
/// database as it was.
fn corrupt(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(error))
}

/// Stored data that is not what its format says, in words.
fn corrupt_text(message: &str) -> rusqlite::Error {
    corrupt(std::io::Error::other(message.to_owned()))
}

/// A kind of database, as a server's upgrade asks about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseKind {
    Control,
    Conversation,
}

impl DatabaseKind {
    pub(crate) fn schema(self) -> &'static Schema {
        match self {
            Self::Control => &CONTROL,
            Self::Conversation => &CONVERSATION,
        }
    }
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
pub fn schema_fit(path: &Path, kind: DatabaseKind) -> Result<SchemaFit, StorageError> {
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
    Ok(match kind.schema().current(recorded) {
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
pub(crate) fn version_of(sql: &str) -> i32 {
    let digest = Sha256::digest(sql.as_bytes());
    let bits = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) >> 1;
    let version = i32::try_from(bits).expect("31 bits fit");
    version.max(1)
}

impl Schema {
    pub(crate) fn version(&self) -> i32 {
        version_of(self.sql)
    }

    #[cfg(feature = "testing")]
    pub(crate) fn sql(&self) -> &'static str {
        self.sql
    }

    /// The text of each schema in the history, oldest first.
    #[cfg(feature = "testing")]
    pub(crate) fn history(&self) -> impl Iterator<Item = &'static str> {
        self.history.iter().map(|shipped| shipped.sql)
    }

    /// The migrations that bring a database of `recorded`'s schema to this
    /// one, in order; none for a version the history does not hold.
    fn migrations(&self, recorded: i32) -> Option<&'static [Shipped]> {
        let from = self
            .history
            .iter()
            .position(|shipped| version_of(shipped.sql) == recorded)?;
        Some(&self.history[from..])
    }

    /// Gives a file that holds no database yet what it takes before its
    /// journal mode is set: the pragmas of [`Schema::initial`].
    pub(crate) fn prepare_new(&self, connection: &Connection) -> Result<(), StorageError> {
        let pages: i64 = connection.pragma_query_value(None, "page_count", |row| row.get(0))?;
        if pages == 0 {
            connection.execute_batch(self.initial)?;
        }
        Ok(())
    }

    /// Gives `connection`'s database this schema, migrating one of an earlier
    /// release's, for a database whose migrations re-encode sealed values with
    /// `seals`: the control database.
    pub(crate) fn apply_sealed(
        &self,
        connection: &mut Connection,
        path: &Path,
        seals: &dyn ResealSealed,
    ) -> Result<(), StorageError> {
        self.migrate(connection, path, &NoBlobs, seals)
    }

    /// Gives `connection`'s database this schema, migrating one of an earlier
    /// release's, for a database that holds no sealed values: a conversation's.
    pub(crate) fn apply(
        &self,
        connection: &mut Connection,
        path: &Path,
        blobs: &dyn ReadBlob,
    ) -> Result<(), StorageError> {
        self.migrate(connection, path, blobs, &NoSealed)
    }

    fn migrate(
        &self,
        connection: &mut Connection,
        path: &Path,
        blobs: &dyn ReadBlob,
        seals: &dyn ResealSealed,
    ) -> Result<(), StorageError> {
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
                    Migration::Code(migrate) => migrate(&transaction, blobs),
                    Migration::Reseal(migrate) => migrate(&transaction, seals),
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
    pub(crate) fn current(&self, recorded: i32) -> Option<bool> {
        if recorded == self.version() {
            return Some(true);
        }
        self.migrations(recorded).map(|_| false)
    }
}
