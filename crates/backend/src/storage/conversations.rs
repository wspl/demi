//! The conversation databases (`storage.md` § Conversation state and
//! transactions): one file per conversation under `conversations/`, holding
//! its agent tree. A conversation's handle is stable; the writer connection
//! behind it opens on demand, on a thread of its own, and at most a fixed
//! number of writers are open at once, the least recently used closing
//! first. Closing one loses nothing: the next call opens it again. A read
//! that needs no live session takes a short-lived read-only connection on
//! the blocking pool instead, which takes no writer and never creates a
//! file.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use demi_gates::KeyedSerialGate;
use demi_web_api::ids::ConversationId;
use hashlink::LruCache;
use rusqlite::{Connection, OpenFlags};

use super::{StorageError, schema, sqlite};

/// The most writer connections open at once.
pub(crate) const MAX_WRITERS: NonZeroUsize = NonZeroUsize::new(64).expect("64 is not zero");

/// Every conversation's database. Cloning it is cheap.
#[derive(Clone)]
pub(crate) struct ConversationStores(Arc<Stores>);

struct Stores {
    directory: PathBuf,
    /// A `std` mutex: the edge and the shard threads share the writers, and
    /// each section looks up, inserts or takes writers without awaiting.
    writers: Mutex<Writers>,
    /// Opening a database takes its file's turn, so two first calls cannot
    /// both create it and give it its schema.
    opening: KeyedSerialGate<DatabaseFile>,
}

struct Writers {
    open: LruCache<DatabaseFile, tokio_rusqlite::Connection>,
    closed: bool,
}

/// The file of a conversation's database: its id in lowercase. Ids never
/// differ only in case (the conversation index compares them without case),
/// so each conversation has one file, whatever the file system does with
/// case and whichever case a caller spells the id in.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DatabaseFile(String);

impl DatabaseFile {
    fn of(conversation: &ConversationId) -> Self {
        Self(conversation.as_str().to_ascii_lowercase())
    }

    fn path(&self, directory: &Path) -> PathBuf {
        directory.join(format!("{}.sqlite", self.0))
    }
}

/// One conversation's database, as a handle that stays valid while its
/// writer closes and opens again.
#[derive(Clone)]
pub(crate) struct ConversationDb {
    stores: Arc<Stores>,
    file: DatabaseFile,
}

impl ConversationStores {
    /// The databases in `directory`, which is created if it is missing, with
    /// at most `max_writers` writers open at once.
    pub(crate) async fn open(directory: PathBuf, max_writers: NonZeroUsize) -> Result<Self, StorageError> {
        tokio::fs::create_dir_all(&directory).await?;
        Ok(Self(Arc::new(Stores {
            directory,
            writers: Mutex::new(Writers {
                open: LruCache::new(max_writers.get()),
                closed: false,
            }),
            opening: KeyedSerialGate::new(),
        })))
    }

    pub(crate) fn db(&self, conversation: &ConversationId) -> ConversationDb {
        ConversationDb {
            stores: self.0.clone(),
            file: DatabaseFile::of(conversation),
        }
    }

    /// Runs `work` on a read-only connection of the conversation's database,
    /// in one read transaction, so its statements see one state of the
    /// database. A conversation with no database yet answers `None`.
    pub(crate) async fn read<T: Send + 'static>(
        &self,
        conversation: &ConversationId,
        work: impl FnOnce(&Connection) -> Result<T, StorageError> + Send + 'static,
    ) -> Result<Option<T>, StorageError> {
        if self.0.lock().closed {
            return Err(StorageError::Closed);
        }
        let path = DatabaseFile::of(conversation).path(&self.0.directory);
        tokio::task::spawn_blocking(move || read_cold(&path, work)).await?
    }

    /// Closes every writer and waits for each to close its database; a call
    /// or read after it fails with `Closed`. Answers each writer that did not
    /// close cleanly.
    pub(crate) async fn close(&self) -> Vec<StorageError> {
        let writers: Vec<tokio_rusqlite::Connection> = {
            let mut writers = self.0.lock();
            writers.closed = true;
            writers.open.drain().map(|(_, writer)| writer).collect()
        };
        let mut failures = Vec::new();
        for writer in writers {
            if let Err(error) = sqlite::close(writer).await {
                failures.push(error);
            }
        }
        failures
    }

    #[cfg(test)]
    fn open_writers(&self) -> usize {
        self.0.lock().open.len()
    }
}

impl Stores {
    fn lock(&self) -> MutexGuard<'_, Writers> {
        // No section panics while it holds the lock, so a poisoned one still
        // holds whole writers.
        self.writers.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ConversationDb {
    /// Runs `work` on the conversation's writer connection, on its thread.
    /// The first call opens the writer, and a new database receives its
    /// schema. `work` is synchronous, so a transaction it opens never spans
    /// a wait.
    pub(crate) async fn call<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Connection) -> Result<T, StorageError> + Send + 'static,
    ) -> Result<T, StorageError> {
        let writer = self.writer().await?;
        writer.call(work).await.map_err(sqlite::flatten)
    }

    async fn writer(&self) -> Result<tokio_rusqlite::Connection, StorageError> {
        if let Some(writer) = self.open_writer()? {
            return Ok(writer);
        }
        let _turn = self.stores.opening.acquire(self.file.clone()).await;
        // Another caller may have opened it while this one waited its turn.
        if let Some(writer) = self.open_writer()? {
            return Ok(writer);
        }
        let writer = open_writer(&self.file.path(&self.stores.directory)).await?;
        let mut writers = self.stores.lock();
        if writers.closed {
            // Dropping the writer ends its thread, which closes the database.
            return Err(StorageError::Closed);
        }
        // Beyond the limit, this drops the least recently used writer: calls
        // already queued on it finish on its thread, which then closes the
        // database, and the busy timeout covers their brief overlap with a
        // writer opened again.
        writers.open.insert(self.file.clone(), writer.clone());
        Ok(writer)
    }

    /// The writer, if it is open, as the most recently used.
    fn open_writer(&self) -> Result<Option<tokio_rusqlite::Connection>, StorageError> {
        let mut writers = self.stores.lock();
        if writers.closed {
            return Err(StorageError::Closed);
        }
        Ok(writers.open.get(&self.file).cloned())
    }
}

async fn open_writer(path: &Path) -> Result<tokio_rusqlite::Connection, StorageError> {
    let writer = tokio_rusqlite::Connection::open(path).await?;
    writer
        .call(|connection| {
            sqlite::configure(connection)?;
            schema::CONVERSATION.to_latest(connection)?;
            Ok(())
        })
        .await
        .map_err(sqlite::flatten)?;
    Ok(writer)
}

fn read_cold<T>(
    path: &Path,
    work: impl FnOnce(&Connection) -> Result<T, StorageError>,
) -> Result<Option<T>, StorageError> {
    if !path.try_exists()? {
        return Ok(None);
    }
    let mut connection =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    connection.busy_timeout(sqlite::BUSY_TIMEOUT)?;
    let transaction = connection.transaction()?;
    // A writer that has just created the file may not have given it its
    // schema yet; such a database holds nothing yet.
    let version: i64 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == 0 {
        return Ok(None);
    }
    work(&transaction).map(Some)
}

#[cfg(test)]
mod tests {
    use rusqlite::{OptionalExtension, params};

    use super::*;

    fn conversation(number: u8) -> ConversationId {
        ConversationId::try_from(format!("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a{number:02x}")).unwrap()
    }

    /// The root node's row, as a tree store writes it.
    async fn write_root(db: &ConversationDb, conversation: &ConversationId, state: &str) -> Result<(), StorageError> {
        let (id, state) = (conversation.as_str().to_owned(), state.to_owned());
        db.call(move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "INSERT INTO nodes (id, parent_id, description, profile, spawned_at, can_spawn, delivered,
                                    state, block_count, command_revision, output_revision)
                 VALUES (?1, NULL, '', NULL, 1, 1, 0, ?2, 0, 0, 0)
                 ON CONFLICT (id) DO UPDATE SET state = excluded.state",
                params![id, state],
            )?;
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    fn root_state(connection: &Connection) -> Result<Option<String>, StorageError> {
        let state = connection
            .query_row("SELECT state FROM nodes WHERE parent_id IS NULL", [], |row| row.get(0))
            .optional()?;
        Ok(state)
    }

    #[tokio::test]
    async fn writers_stay_bounded_and_cold_reads_take_none_and_create_no_file() {
        let data = tempfile::tempdir().unwrap();
        let directory = data.path().join("conversations");
        let stores = ConversationStores::open(directory.clone(), NonZeroUsize::new(2).unwrap())
            .await
            .unwrap();
        let (a, b, c) = (conversation(1), conversation(2), conversation(3));
        let first = stores.db(&a);
        write_root(&first, &a, "a").await.unwrap();
        write_root(&stores.db(&b), &b, "b").await.unwrap();
        assert_eq!(stores.open_writers(), 2);

        assert_eq!(stores.read(&c, root_state).await.unwrap(), None);
        assert!(!directory.join(format!("{c}.sqlite")).exists());
        assert_eq!(stores.read(&b, root_state).await.unwrap(), Some(Some("b".to_owned())));
        assert_eq!(stores.open_writers(), 2);

        // A third writer closes the least recently used, a's. The handle
        // handed out before opens it again, and nothing was lost.
        write_root(&stores.db(&c), &c, "c").await.unwrap();
        assert_eq!(stores.open_writers(), 2);
        assert_eq!(first.call(|connection| root_state(connection)).await.unwrap(), Some("a".to_owned()));
        write_root(&first, &a, "a2").await.unwrap();
        assert_eq!(stores.open_writers(), 2);

        // Another spelling of an id names the same database.
        let upper = ConversationId::try_from(a.as_str().to_uppercase()).unwrap();
        assert_eq!(stores.read(&upper, root_state).await.unwrap(), Some(Some("a2".to_owned())));

        assert!(stores.close().await.is_empty());
        assert_eq!(stores.open_writers(), 0);
        assert!(matches!(first.call(|_| Ok(())).await, Err(StorageError::Closed)));
        assert!(matches!(stores.read(&a, |_| Ok(())).await, Err(StorageError::Closed)));

        // After a restart, a cold read finds every database as it was left.
        let reopened = ConversationStores::open(directory, MAX_WRITERS).await.unwrap();
        for (conversation, state) in [(&a, "a2"), (&b, "b"), (&c, "c")] {
            assert_eq!(reopened.read(conversation, root_state).await.unwrap(), Some(Some(state.to_owned())));
        }
        assert_eq!(reopened.open_writers(), 0);
    }

    #[tokio::test]
    async fn first_calls_at_once_open_one_writer_and_both_commit() {
        let data = tempfile::tempdir().unwrap();
        let stores = ConversationStores::open(data.path().join("conversations"), MAX_WRITERS)
            .await
            .unwrap();
        let a = conversation(1);
        let (one, other) = (stores.db(&a), stores.db(&a));
        let (first, second) = tokio::join!(write_root(&one, &a, "first"), write_root(&other, &a, "second"));
        first.unwrap();
        second.unwrap();
        assert_eq!(stores.open_writers(), 1);
    }
}
