//! The conversation databases: one file per conversation under `conversations/`,
//! holding its agent tree.
//!
//! - A conversation's handle is stable. The writer connection behind it opens on
//!   demand, on a thread of its own, and at most a fixed number of writers are
//!   open at once, the least recently used closing first. Closing one loses
//!   nothing: the next call opens it again.
//! - A read that needs no live session takes a short-lived read-only connection
//!   on the blocking pool instead, which takes no writer and never creates a file.
//! - A test may hold the commits of the checkpoints (`CommitHold`) and the
//!   migrations of the databases (`MigrationHold`).

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bytes::Bytes;
use demi_shared_gates::KeyedSerialGate;
use demi_shared_types::BlobRef;
use demi_web_api_protocol::ids::ConversationId;
use hashlink::LruCache;
use rusqlite::{Connection, OpenFlags};

use super::{StorageError, schema, sqlite};

/// The most writer connections open at once.
///
/// Bounded because each writer is an open database connection, and the
/// operating system limits open files.
pub const MAX_WRITERS: NonZeroUsize = NonZeroUsize::new(64).expect("64 is not zero");

/// How a migration of a conversation's database reads a blob its values
/// name, in the conversation owner's namespace; none when the namespace
/// does not hold it. It is called on the database's thread, which waits
/// for it, never on an async task.
pub trait ConversationBlobs: Send + Sync {
    fn read(&self, conversation: &ConversationId, blob: &BlobRef) -> Result<Option<Bytes>, String>;
}

impl ConversationBlobs for schema::NoBlobs {
    fn read(&self, _: &ConversationId, blob: &BlobRef) -> Result<Option<Bytes>, String> {
        schema::ReadBlob::read(self, blob)
    }
}

/// One conversation's blobs, as its database's migration reads them.
struct BlobsOf<'a> {
    blobs: &'a dyn ConversationBlobs,
    conversation: ConversationId,
}

impl schema::ReadBlob for BlobsOf<'_> {
    fn read(&self, blob: &BlobRef) -> Result<Option<Bytes>, String> {
        self.blobs.read(&self.conversation, blob)
    }
}

/// Every conversation's database. Cloning it is cheap.
#[derive(Clone)]
pub struct ConversationStores(Arc<Stores>);

struct Stores {
    directory: PathBuf,
    /// Where a migration reads the blobs a conversation's values name.
    blobs: Arc<dyn ConversationBlobs>,
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
pub struct ConversationDb {
    stores: Arc<Stores>,
    file: DatabaseFile,
    conversation: ConversationId,
}

impl ConversationStores {
    /// The databases in `directory`, which is created if it is missing, with
    /// at most `max_writers` writers open at once.
    pub async fn open(
        directory: PathBuf,
        max_writers: NonZeroUsize,
        blobs: Arc<dyn ConversationBlobs>,
    ) -> Result<Self, StorageError> {
        tokio::fs::create_dir_all(&directory).await?;
        Ok(Self(Arc::new(Stores {
            directory,
            blobs,
            writers: Mutex::new(Writers {
                open: LruCache::new(max_writers.get()),
                closed: false,
            }),
            opening: KeyedSerialGate::new(),
        })))
    }

    pub fn db(&self, conversation: &ConversationId) -> ConversationDb {
        ConversationDb {
            stores: self.0.clone(),
            file: DatabaseFile::of(conversation),
            conversation: conversation.clone(),
        }
    }

    /// Runs `work` on a read-only connection of the conversation's database,
    /// in one read transaction, so its statements see one state of the
    /// database. A conversation with no database yet answers `None`.
    pub async fn read<T: Send + 'static>(
        &self,
        conversation: &ConversationId,
        work: impl FnOnce(&Connection) -> Result<T, StorageError> + Send + 'static,
    ) -> Result<Option<T>, StorageError> {
        self.db(conversation).read(work).await
    }

    /// Closes every writer and waits for each to close its database; a call
    /// or read after it fails with `Closed`. Answers each writer that did not
    /// close cleanly.
    pub async fn close(&self) -> Vec<StorageError> {
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

    /// Removes the conversation's database: its writer closes, under the file's
    /// opening turn so none opens meanwhile, and its file goes with the write-ahead
    /// log and the shared memory beside it.
    ///
    /// A file already gone is no failure, so a deletion that a start finishes may
    /// remove it again.
    pub async fn remove(&self, conversation: &ConversationId) -> Result<(), StorageError> {
        let file = DatabaseFile::of(conversation);
        let _turn = self.0.opening.acquire(file.clone()).await;
        let writer = self.0.lock().open.remove(&file);
        if let Some(writer) = writer {
            sqlite::close(writer).await?;
        }
        let database = file.path(&self.0.directory);
        for suffix in ["", "-wal", "-shm"] {
            let mut path = database.clone().into_os_string();
            path.push(suffix);
            match tokio::fs::remove_file(PathBuf::from(path)).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
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
    /// The file of the conversation `other`'s database, for a copy of its
    /// rows into this one; none when it is this one's.
    pub(crate) fn file_of(&self, other: &ConversationId) -> Option<PathBuf> {
        let file = DatabaseFile::of(other);
        (file != self.file).then(|| file.path(&self.stores.directory))
    }

    /// Runs `work` on the conversation's writer connection, on its thread.
    /// The first call opens the writer, and a new database receives its
    /// schema. `work` is synchronous, so a transaction it opens never spans
    /// a wait.
    pub async fn call<T: Send + 'static>(
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
        let writer = open_writer(
            &self.file.path(&self.stores.directory),
            self.stores.blobs.clone(),
            self.conversation.clone(),
        )
        .await?;
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

    /// Runs `work` on a read-only connection of the database, in one read
    /// transaction, so its statements see one state of the database. A
    /// conversation with no database yet answers `None`.
    pub async fn read<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Connection) -> Result<T, StorageError> + Send + 'static,
    ) -> Result<Option<T>, StorageError> {
        if self.stores.lock().closed {
            return Err(StorageError::Closed);
        }
        let path = self.file.path(&self.stores.directory);
        let first = {
            let path = path.clone();
            tokio::task::spawn_blocking(move || read_current(&path, work)).await??
        };
        let work = match first {
            Cold::Read(value) => return Ok(value),
            Cold::Stale(work) => work,
        };
        // A database of an earlier release's schema, not opened since the upgrade,
        // migrates as an open would, then is read.
        //
        // The migration takes the file's opening turn, as an open does: a read that
        // comes while the conversation's open migrates the database waits for that
        // migration, however long it takes, instead of migrating beside it and
        // giving up on its write lock after the busy timeout, which left the
        // conversation listed without its facts (0.1.22).
        //
        let _turn = self.stores.opening.acquire(self.file.clone()).await;
        let (blobs, conversation) = (self.stores.blobs.clone(), self.conversation.clone());
        tokio::task::spawn_blocking(move || {
            let blobs = BlobsOf {
                blobs: &*blobs,
                conversation,
            };
            migrate_and_read(&path, &blobs, work)
        })
        .await?
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

/// Opens the writer of the database at `path`, under its file's opening
/// turn, which the caller holds.
async fn open_writer(
    path: &Path,
    blobs: Arc<dyn ConversationBlobs>,
    conversation: ConversationId,
) -> Result<tokio_rusqlite::Connection, StorageError> {
    let writer = tokio_rusqlite::Connection::open(path).await?;
    let path = path.to_owned();
    writer
        .call(move |connection| {
            let blobs = BlobsOf {
                blobs: &*blobs,
                conversation,
            };
            migrate(connection, &path, &blobs)
        })
        .await
        .map_err(sqlite::flatten)?;
    Ok(writer)
}

/// Configures `connection` and gives its database the conversation schema,
/// migrating one of an earlier release's, under the file's opening turn,
/// which the caller holds.
fn migrate(
    connection: &mut Connection,
    path: &Path,
    blobs: &BlobsOf<'_>,
) -> Result<(), StorageError> {
    sqlite::configure(connection)?;
    schema::CONVERSATION.apply(connection, path, blobs)
}

/// What a read of a database of the current schema answered, or the work of
/// one that found an earlier release's schema, which must migrate first.
enum Cold<T, W> {
    Read(Option<T>),
    Stale(W),
}

/// Runs `work` on a read-only connection of the database at `path` when it
/// holds the current schema; answers `None` when it holds nothing yet.
fn read_current<T, W>(path: &Path, work: W) -> Result<Cold<T, W>, StorageError>
where
    W: FnOnce(&Connection) -> Result<T, StorageError>,
{
    if !path.try_exists()? {
        return Ok(Cold::Read(None));
    }
    let mut connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(sqlite::BUSY_TIMEOUT)?;
    let transaction = connection.transaction()?;
    // A writer that has just created the file may not have given it its
    // schema yet; such a database holds nothing yet.
    let version: i32 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == 0 {
        return Ok(Cold::Read(None));
    }
    match schema::CONVERSATION.current(version) {
        Some(true) => work(&transaction).map(|value| Cold::Read(Some(value))),
        Some(false) => Ok(Cold::Stale(work)),
        None => Err(StorageError::OtherSchema {
            path: path.to_owned(),
        }),
    }
}

/// Reads the database at `path` under its file's opening turn, which the caller
/// holds, migrating it first when it still holds an earlier release's schema.
///
/// An open that held the turn before may have migrated it meanwhile; then it
/// reads as it is, and no writer's transaction is in the way, since a writer
/// opens only once the database is migrated.
fn migrate_and_read<T, W>(
    path: &Path,
    blobs: &BlobsOf<'_>,
    work: W,
) -> Result<Option<T>, StorageError>
where
    W: FnOnce(&Connection) -> Result<T, StorageError>,
{
    let work = match read_current(path, work)? {
        Cold::Read(value) => return Ok(value),
        Cold::Stale(work) => work,
    };
    // The file is there: removing it takes the opening turn too.
    let mut connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    migrate(&mut connection, path, blobs)?;
    let transaction = connection.transaction()?;
    work(&transaction).map(Some)
}
