//! The core store: every read and write of a core's own database,
//! `core.sqlite` in its data directory, each one closure and one transaction
//! on the database's own thread, and the databases beside it.
//!
//! ```text
//! data directory
//! +-- core.sqlite                  the principal's records
//! +-- conversations/<id>.sqlite    one agent tree per conversation
//! +-- search.sqlite                the search index, derived
//! ```
//!
//! A core serves one principal, whose id seals what the store seals, so no
//! row names it.
//!
//! Operations take owned, serializable input, so no caller holds a
//! transaction open across a wait.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use demi_shared_types::{Clock, Timestamp};
use demi_web_api_protocol::ids::UserId;
use rusqlite::Connection;
use tokio::sync::broadcast;

use super::conversations::{ConversationBlobs, ConversationStores, MAX_WRITERS};
use super::search::SearchIndex;
use super::seal::VaultKey;
use super::workspaces;
use super::usage::UsageWritten;
use super::{StorageError, schema, sqlite};

/// The name of a core's own database in its data directory.
pub const CORE_DATABASE: &str = "core.sqlite";
/// The name of the directory of a core's conversation databases in its data
/// directory.
pub const CONVERSATION_DATABASES: &str = "conversations";
/// The name of a core's search index in its data directory.
pub const SEARCH_INDEX: &str = "search.sqlite";

/// What a core keeps, in the data directory it was given. Cloning it is
/// cheap.
#[derive(Clone)]
pub struct CoreStore {
    db: tokio_rusqlite::Connection,
    clock: Arc<dyn Clock>,
    principal: UserId,
    /// What its credentials, its entries' keys and its plugins' and
    /// schedules' secrets are sealed under.
    key: VaultKey,
    directory: PathBuf,
    conversations: ConversationStores,
    search: SearchIndex,
    /// Each usage ledger row as it is written, for whoever shows its
    /// conversation's cost (`usage.rs`).
    usage_written: broadcast::Sender<UsageWritten>,
}

/// How many written rows wait for a follower that fell behind before it
/// misses the oldest.
///
/// Bounded because the rows wait in memory for each follower, and a follower
/// that never reads would grow them without end.
const USAGE_WAITING: usize = 1024;

impl CoreStore {
    /// Opens the core's data in `directory`, making the directory and the
    /// database when there is none, with the workspace a new core starts
    /// with, and migrating one of an earlier schema. `blobs` is where the
    /// migrations of its conversation databases read the blobs they name.
    pub async fn open(
        directory: &Path,
        principal: UserId,
        key: VaultKey,
        clock: Arc<dyn Clock>,
        blobs: Arc<ConversationBlobs>,
    ) -> Result<Self, StorageError> {
        tokio::fs::create_dir_all(directory).await?;
        let path = directory.join(CORE_DATABASE);
        let db = tokio_rusqlite::Connection::open(&path).await?;
        let conversations =
            ConversationStores::open(directory.join(CONVERSATION_DATABASES), MAX_WRITERS, blobs).await?;
        let store = Self {
            db,
            clock,
            principal,
            key,
            directory: directory.to_owned(),
            conversations,
            search: SearchIndex::new(directory.join(SEARCH_INDEX)),
            usage_written: broadcast::channel(USAGE_WAITING).0,
        };
        store
            .call(move |connection, now| {
                schema::CORE.prepare_new(connection)?;
                sqlite::configure(connection)?;
                schema::CORE.apply(connection, &path, &())?;
                let transaction = connection.transaction()?;
                let has_workspace: bool =
                    transaction.query_row("SELECT EXISTS (SELECT 1 FROM workspaces)", [], |row| row.get(0))?;
                if !has_workspace {
                    workspaces::insert_first(&transaction, now)?;
                }
                transaction.commit()?;
                Ok(())
            })
            .await?;
        Ok(store)
    }

    /// The core's conversation databases.
    pub fn databases(&self) -> &ConversationStores {
        &self.conversations
    }

    /// The core's search index.
    pub fn search(&self) -> &SearchIndex {
        &self.search
    }

    /// The principal the core serves.
    pub fn principal(&self) -> &UserId {
        &self.principal
    }

    /// The key the core's secrets are sealed under.
    pub fn key(&self) -> &VaultKey {
        &self.key
    }

    /// The data directory the store keeps its databases in.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Each usage ledger row written from now on.
    pub fn usage_written(&self) -> broadcast::Receiver<UsageWritten> {
        self.usage_written.subscribe()
    }

    pub(crate) fn announce_usage(&self, written: UsageWritten) {
        // No follower is no one to tell.
        let _ = self.usage_written.send(written);
    }

    /// The wall-clock time the records it writes carry.
    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }

    /// Closes the conversation databases, then the core database, and
    /// answers what failed; operations after it fail with `Closed`.
    pub async fn close(&self) -> Vec<StorageError> {
        let mut failures = self.conversations.close().await;
        if let Err(error) = sqlite::close(self.db.clone()).await {
            failures.push(error);
        }
        failures
    }

    /// Runs `work` on the database thread with the time of the operation, in
    /// the whole milliseconds storage keeps.
    pub(crate) async fn call<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Connection, Timestamp) -> Result<T, StorageError> + Send + 'static,
    ) -> Result<T, StorageError> {
        let clock = self.clock.clone();
        self.db
            .call(move |connection| work(connection, clock.now()))
            .await
            .map_err(sqlite::flatten)
    }
}
