//! What the crate's test reads of each database's schemas, behind the
//! `testing` feature: the control database's and a conversation's, whose
//! schema is the core store's.

use demi_core_store::schema::{CONVERSATION, CORE, NoBlobs};
use demi_web_api_protocol::ids::ProviderId;
use rusqlite::Connection;

use super::StorageError;
use super::schema::{CONTROL, ControlSteps};
use super::DatabaseKind;

/// The text of `kind`'s current schema.
pub fn current(kind: DatabaseKind) -> &'static str {
    match kind {
        DatabaseKind::Control => CONTROL.sql,
        DatabaseKind::Core => CORE.sql,
        DatabaseKind::Conversation => CONVERSATION.sql,
    }
}

/// The text of each schema in `kind`'s history, oldest first.
pub fn history(kind: DatabaseKind) -> Vec<&'static str> {
    match kind {
        DatabaseKind::Control => CONTROL.history_sql().collect(),
        DatabaseKind::Core => CORE.history_sql().collect(),
        DatabaseKind::Conversation => CONVERSATION.history_sql().collect(),
    }
}

/// The version a database of the schema `sql` records.
pub fn version_of(sql: &str) -> i32 {
    demi_core_store::schema::version_of(sql)
}

/// Gives the database of `connection` `kind`'s current schema, as an open
/// does, with no blob to read and no vault to seal again.
pub fn migrate(kind: DatabaseKind, connection: &mut Connection) -> Result<(), StorageError> {
    let path = std::path::Path::new("test");
    match kind {
        DatabaseKind::Control => CONTROL.apply(connection, path, &NoSeals),
        DatabaseKind::Core => CORE.apply(connection, path, &()),
        DatabaseKind::Conversation => CONVERSATION.apply(connection, path, &NoBlobs),
    }
}

/// Gives the control database of `connection` its current schema, as an
/// open does, with `seals` sealing again what its steps re-encode.
pub fn migrate_control(connection: &mut Connection, seals: &(dyn ControlSteps + 'static)) -> Result<(), StorageError> {
    CONTROL.apply(connection, std::path::Path::new("test"), seals)
}

/// No vault: a step that re-encodes a sealed value fails.
struct NoSeals;

impl ControlSteps for NoSeals {
    fn data_directory(&self) -> &std::path::Path {
        std::path::Path::new("test")
    }

    fn provider_config(&self, id: &ProviderId, _: &str, _: &[u8]) -> Result<Vec<u8>, String> {
        Err(format!("no key opens the configuration of entry {id} here"))
    }

    fn schedule_secret(&self, id: &str, _: &[u8]) -> Result<Vec<u8>, String> {
        Err(format!("no key opens the secret of schedule {id} here"))
    }

    fn provider_endpoint_on_device(
        &self,
        id: &ProviderId,
        _: &[u8],
        _: &dyn Fn(&str) -> bool,
    ) -> Result<Option<(String, Vec<u8>)>, String> {
        Err(format!("no key opens the configuration of entry {id} here"))
    }

    fn provider_config_format_3(&self, id: &ProviderId, _: &[u8]) -> Result<Option<Vec<u8>>, String> {
        Err(format!("no key opens the configuration of entry {id} here"))
    }
}
