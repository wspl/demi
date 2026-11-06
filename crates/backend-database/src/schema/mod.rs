//! The control database's schema and its history, and the kinds of database
//! a server's upgrade asks about.
//!
//! ```text
//! schema/
//!   control.sql                     the current schema
//!   control/                        its history, oldest first:
//!     mod.rs                          the list of its steps
//!     from_<release>.sql              a schema a server may hold
//!     from_<release>.rs               the step from it to the next
//! ```
//!
//! The machinery, and the rules every step follows, are the core store's
//! (`crates/core-store/src/schema/mod.rs`). A step of this history is handed
//! the vault, which alone seals again what it re-encodes ([`ResealSealed`]).
//!
//! The test of the crate (`tests/backend-database/`) takes every step of the
//! history from an empty database of its schema and compares the result with
//! a new database of the current one.

use std::path::Path;

use demi_core_store::StorageError;
use demi_core_store::schema::{CONVERSATION, Schema, SchemaFit, schema_fit};
use demi_web_api_protocol::ids::ProviderId;

pub(crate) use demi_core_store::schema::{corrupt, corrupt_text};

mod control;

/// A step of the control database's history.
pub(crate) type Migration = demi_core_store::schema::Migration<dyn ResealSealed>;
/// A schema of the control database's history.
pub(crate) type Shipped = demi_core_store::schema::Shipped<dyn ResealSealed>;

/// The control database's schema. Its history holds every schema a server
/// may hold before the current one (`upgrades.md` § A development build on
/// a server).
pub const CONTROL: Schema<dyn ResealSealed> = Schema {
    initial: "",
    sql: include_str!("control.sql"),
    history: control::HISTORY,
};

/// A kind of database, as a server's upgrade asks about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseKind {
    Control,
    Conversation,
}

/// What this build would do with the database of `kind` at `path` as it
/// opens it. The database is read, never written.
pub fn database_fit(path: &Path, kind: DatabaseKind) -> Result<SchemaFit, StorageError> {
    match kind {
        DatabaseKind::Control => schema_fit(path, &CONTROL),
        DatabaseKind::Conversation => schema_fit(path, &CONVERSATION),
    }
}

/// What the control migrations that re-encode sealed values work with: the
/// vault, which alone holds the key they are sealed under and knows their
/// format.
pub trait ResealSealed: Send + Sync {
    /// Entry `id` of `family`'s configuration `sealed`, opened, decoded and
    /// checked in the format the schema before names, and sealed again in
    /// the format of the schema after it; an error, which never quotes the
    /// configuration, says why it cannot be read.
    fn provider_config(&self, id: &ProviderId, family: &str, sealed: &[u8]) -> Result<Vec<u8>, String>;

    /// The URL secret `sealed` for the automation `id`, opened and sealed
    /// again for the schedule of the same id; an error, which never quotes
    /// the secret, says why it cannot be opened.
    fn schedule_secret(&self, id: &str, sealed: &[u8]) -> Result<Vec<u8>, String>;

    /// Entry `id`'s configuration `sealed`, opened: when its endpoint's host
    /// is a device's id that `owned` accepts, that device and the
    /// configuration sealed again with the endpoint on the device itself
    /// (`schema/control/from_0_1_5860.rs`); none for any other endpoint. An
    /// error, which never quotes the configuration, says why it cannot be
    /// read.
    fn provider_endpoint_on_device(
        &self,
        id: &ProviderId,
        sealed: &[u8],
        owned: &dyn Fn(&str) -> bool,
    ) -> Result<Option<(String, Vec<u8>)>, String>;

    /// Entry `id`'s configuration `sealed`, opened: one of format 2 sealed
    /// again in format 3, without its image and video shapes and a
    /// compatible entry's image and video models
    /// (`schema/control/from_0_1_5979_reply_block_shares.rs`); none for one of format 3. An
    /// error, which never quotes the configuration, says why it cannot be
    /// read.
    fn provider_config_format_3(&self, id: &ProviderId, sealed: &[u8]) -> Result<Option<Vec<u8>>, String>;
}

