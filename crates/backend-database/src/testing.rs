//! What the crate's test reads of each database's schemas, behind the
//! `testing` feature.

use rusqlite::Connection;

use super::StorageError;
use super::schema::{DatabaseKind, NoBlobs};

/// The text of `kind`'s current schema.
pub fn current(kind: DatabaseKind) -> &'static str {
    kind.schema().sql()
}

/// The text of each schema in `kind`'s history, oldest first.
pub fn history(kind: DatabaseKind) -> Vec<&'static str> {
    kind.schema().history().collect()
}

/// The version a database of the schema `sql` records.
pub fn version_of(sql: &str) -> i32 {
    super::schema::version_of(sql)
}

/// Gives the database of `connection` `kind`'s current schema, as an open
/// does, with no blob to read and no vault to seal again.
pub fn migrate(kind: DatabaseKind, connection: &mut Connection) -> Result<(), StorageError> {
    kind.schema().apply(connection, std::path::Path::new("test"), &NoBlobs)
}
