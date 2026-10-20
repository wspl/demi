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
use rusqlite::{Connection, OpenFlags, Transaction};

use super::{StorageError, schema, sqlite};

/// The most writer connections open at once.
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
    /// The hold a test put on the checkpoints' commits. A `std` mutex: the
    /// test's thread sets it and the shard threads read it as a save
    /// starts, without awaiting.
    #[cfg(feature = "testing")]
    hold: Mutex<Option<Arc<Hold>>>,
    /// The hold a test put on the migrations, as `hold`.
    #[cfg(feature = "testing")]
    migrations: Mutex<Option<Arc<MigrationHolds>>>,
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
            #[cfg(feature = "testing")]
            hold: Mutex::new(None),
            #[cfg(feature = "testing")]
            migrations: Mutex::new(None),
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

    #[cfg(test)]
    fn open_writers(&self) -> usize {
        self.0.lock().open.len()
    }

    /// Holds every commit of a checkpoint or of a stored attachment from
    /// now on, until the hold is released.
    #[cfg(feature = "testing")]
    pub fn hold_commits(&self) -> CommitHold {
        let (hold, waiting) = Hold::new();
        *self.0.hold.lock().unwrap_or_else(PoisonError::into_inner) = Some(hold.clone());
        CommitHold { hold, waiting }
    }

    /// Holds every migration of a conversation's database from now on,
    /// until the hold is released.
    #[cfg(feature = "testing")]
    pub fn hold_migrations(&self) -> MigrationHold {
        let (hold, waiting) = Hold::new();
        let (reads, reading) = tokio::sync::watch::channel(0);
        let holds = Arc::new(MigrationHolds { hold, reads });
        *self
            .0
            .migrations
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(holds.clone());
        MigrationHold {
            holds,
            waiting,
            reading,
        }
    }
}

impl Stores {
    fn lock(&self) -> MutexGuard<'_, Writers> {
        // No section panics while it holds the lock, so a poisoned one still
        // holds whole writers.
        self.writers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Where the next migration starts.
    fn migration_point(&self) -> MigrationPoint {
        MigrationPoint {
            #[cfg(feature = "testing")]
            holds: self
                .migrations
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        }
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
            self.stores.migration_point(),
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
        // Only a read that finds the turn taken waits for it, which a test's hold
        // counts (`MigrationHold::until_reads_wait`).
        let point = self.stores.migration_point();
        let _turn = match self.stores.opening.try_acquire(self.file.clone()) {
            Some(turn) => turn,
            None => {
                point.read_waits();
                self.stores.opening.acquire(self.file.clone()).await
            }
        };
        let (blobs, conversation) = (self.stores.blobs.clone(), self.conversation.clone());
        tokio::task::spawn_blocking(move || {
            let blobs = BlobsOf {
                blobs: &*blobs,
                conversation,
            };
            migrate_and_read(&path, &blobs, &point, work)
        })
        .await?
    }

    /// Where a save's transaction, or a stored attachment's, commits.
    pub fn commit_point(&self) -> CommitPoint {
        CommitPoint {
            #[cfg(feature = "testing")]
            hold: self
                .stores
                .hold
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        }
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

/// Where a save's transaction, or a stored attachment's, commits: at once,
/// unless a test holds the commits there (`CommitHold`).
pub struct CommitPoint {
    #[cfg(feature = "testing")]
    hold: Option<Arc<Hold>>,
}

impl CommitPoint {
    /// Commits `transaction`, on its writer's thread.
    pub fn commit(self, transaction: Transaction<'_>) -> rusqlite::Result<()> {
        #[cfg(feature = "testing")]
        {
            if let Some(hold) = self.hold {
                hold.pass();
            }
        }
        transaction.commit()
    }
}

/// A hold on the commits of the conversations' checkpoints and stored
/// attachments, for the scenarios that stop a save at its commit or a call while
/// its attachment's commit waits (`attachment_commands`).
///
/// From `Backend::hold_commits` until it is released or dropped, a save has
/// written its rows and waits before its transaction commits, on its database's
/// writer thread: where a process that dies inside the save leaves it.
#[cfg(feature = "testing")]
pub struct CommitHold {
    hold: Arc<Hold>,
    waiting: tokio::sync::watch::Receiver<usize>,
}

#[cfg(feature = "testing")]
impl CommitHold {
    /// Waits until `count` commits wait at the hold.
    pub async fn until_waiting(&self, count: usize) {
        let mut waiting = self.waiting.clone();
        waiting
            .wait_for(|waiting| *waiting >= count)
            .await
            .expect("the stores keep the hold while it is held");
    }

    /// Lets the waiting commits and every later one through.
    pub fn release(self) {
        drop(self);
    }
}

/// Dropping a hold releases it, so one that a failed test leaves behind
/// holds nothing.
#[cfg(feature = "testing")]
impl Drop for CommitHold {
    fn drop(&mut self) {
        self.hold.release();
    }
}

/// A hold on the migrations of the conversations' databases, for the scenarios
/// that race a read against a migration.
///
/// From `Backend::held_migrations` until it is released or dropped, a migration
/// holds its database's write lock and waits before it changes the database, on
/// the thread that migrates it: where a long migration, such as one that rewrites
/// every block of a large conversation, keeps its database.
#[cfg(feature = "testing")]
pub struct MigrationHold {
    holds: Arc<MigrationHolds>,
    waiting: tokio::sync::watch::Receiver<usize>,
    reading: tokio::sync::watch::Receiver<usize>,
}

#[cfg(feature = "testing")]
impl MigrationHold {
    /// Waits until `count` migrations wait at the hold.
    pub async fn until_migrating(&self, count: usize) {
        let mut waiting = self.waiting.clone();
        waiting
            .wait_for(|waiting| *waiting >= count)
            .await
            .expect("the stores keep the hold while it is held");
    }

    /// Waits until `count` reads have found their database of an earlier
    /// schema and wait for the file's opening turn, which another holds, to
    /// migrate it, counting those that have done so since.
    pub async fn until_reads_wait(&self, count: usize) {
        let mut reading = self.reading.clone();
        reading
            .wait_for(|reading| *reading >= count)
            .await
            .expect("the stores keep the hold while it is held");
    }

    /// Lets the waiting migrations and every later one through.
    pub fn release(self) {
        drop(self);
    }
}

/// Dropping a hold releases it, so one that a failed test leaves behind
/// holds nothing.
#[cfg(feature = "testing")]
impl Drop for MigrationHold {
    fn drop(&mut self) {
        self.holds.hold.release();
    }
}

/// A migration hold's own hold, and how many reads came to wait for a
/// migration.
#[cfg(feature = "testing")]
struct MigrationHolds {
    hold: Arc<Hold>,
    reads: tokio::sync::watch::Sender<usize>,
}

/// Where a migration of a conversation's database starts changing it: at
/// once, unless a test holds the migrations there (`MigrationHold`).
struct MigrationPoint {
    #[cfg(feature = "testing")]
    holds: Option<Arc<MigrationHolds>>,
}

impl MigrationPoint {
    /// Passes the point, on the thread that migrates the database.
    fn pass(&self) {
        #[cfg(feature = "testing")]
        {
            if let Some(holds) = &self.holds {
                holds.hold.pass();
            }
        }
    }

    /// Counts a read that found its database of an earlier schema and now
    /// waits for the file's opening turn, which another holds, to migrate it.
    fn read_waits(&self) {
        #[cfg(feature = "testing")]
        {
            if let Some(holds) = &self.holds {
                holds.reads.send_modify(|reads| *reads += 1);
            }
        }
    }
}

/// Whether a hold is released, and how many commits or migrations wait at
/// it.
#[cfg(feature = "testing")]
struct Hold {
    /// A `std` mutex with a condition variable: a commit or a migration
    /// waits on its database's thread, which has no runtime to await on, and
    /// the lock is held only to read or set the flag.
    released: Mutex<bool>,
    opened: std::sync::Condvar,
    waiting: tokio::sync::watch::Sender<usize>,
}

#[cfg(feature = "testing")]
impl Hold {
    /// A hold not released yet, and the count of what waits at it.
    fn new() -> (Arc<Self>, tokio::sync::watch::Receiver<usize>) {
        let (waiting, watched) = tokio::sync::watch::channel(0);
        let hold = Arc::new(Self {
            released: Mutex::new(false),
            opened: std::sync::Condvar::new(),
            waiting,
        });
        (hold, watched)
    }

    /// Lets what waits through, and everything that comes later.
    fn release(&self) {
        *self.released.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.opened.notify_all();
    }

    /// Waits, on the database's thread, until the hold is released.
    fn pass(&self) {
        self.waiting.send_modify(|waiting| *waiting += 1);
        let released = self.released.lock().unwrap_or_else(PoisonError::into_inner);
        drop(
            self.opened
                .wait_while(released, |released| !*released)
                .unwrap_or_else(PoisonError::into_inner),
        );
        self.waiting.send_modify(|waiting| *waiting -= 1);
    }
}

/// Opens the writer of the database at `path`, under its file's opening
/// turn, which the caller holds.
async fn open_writer(
    path: &Path,
    blobs: Arc<dyn ConversationBlobs>,
    conversation: ConversationId,
    point: MigrationPoint,
) -> Result<tokio_rusqlite::Connection, StorageError> {
    let writer = tokio_rusqlite::Connection::open(path).await?;
    let path = path.to_owned();
    writer
        .call(move |connection| {
            let blobs = BlobsOf {
                blobs: &*blobs,
                conversation,
            };
            migrate(connection, &path, &blobs, &point)
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
    point: &MigrationPoint,
) -> Result<(), StorageError> {
    sqlite::configure(connection)?;
    schema::CONVERSATION.apply_with(connection, path, blobs, || point.pass())
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
    point: &MigrationPoint,
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
    migrate(&mut connection, path, blobs, point)?;
    let transaction = connection.transaction()?;
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
    async fn write_root(
        db: &ConversationDb,
        conversation: &ConversationId,
        state: &str,
    ) -> Result<(), StorageError> {
        let (id, state) = (conversation.as_str().to_owned(), state.to_owned());
        db.call(move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "INSERT INTO nodes (id, number, parent_id, description, profile, round, started_at, can_spawn,
                                    delivered, state, output_revision)
                 VALUES (?1, 0, NULL, '', NULL, 1, 0, 1, 0, ?2, 0)
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
            .query_row(
                "SELECT state FROM nodes WHERE parent_id IS NULL",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(state)
    }

    #[tokio::test]
    async fn writers_stay_bounded_and_cold_reads_take_none_and_create_no_file() {
        let data = tempfile::tempdir().unwrap();
        let directory = data.path().join("conversations");
        let stores = ConversationStores::open(directory.clone(), NonZeroUsize::new(2).unwrap(), Arc::new(schema::NoBlobs))
            .await
            .unwrap();
        let (a, b, c) = (conversation(1), conversation(2), conversation(3));
        let first = stores.db(&a);
        write_root(&first, &a, "a").await.unwrap();
        write_root(&stores.db(&b), &b, "b").await.unwrap();
        assert_eq!(stores.open_writers(), 2);

        assert_eq!(stores.read(&c, root_state).await.unwrap(), None);
        assert!(!directory.join(format!("{c}.sqlite")).exists());
        assert_eq!(
            stores.read(&b, root_state).await.unwrap(),
            Some(Some("b".to_owned()))
        );
        assert_eq!(stores.open_writers(), 2);

        // A third writer closes the least recently used, a's. The handle
        // handed out before opens it again, and nothing was lost.
        write_root(&stores.db(&c), &c, "c").await.unwrap();
        assert_eq!(stores.open_writers(), 2);
        assert_eq!(
            first
                .call(|connection| root_state(connection))
                .await
                .unwrap(),
            Some("a".to_owned())
        );
        write_root(&first, &a, "a2").await.unwrap();
        assert_eq!(stores.open_writers(), 2);

        // Another spelling of an id names the same database.
        let upper = ConversationId::try_from(a.as_str().to_uppercase()).unwrap();
        assert_eq!(
            stores.read(&upper, root_state).await.unwrap(),
            Some(Some("a2".to_owned()))
        );

        assert!(stores.close().await.is_empty());
        assert_eq!(stores.open_writers(), 0);
        assert!(matches!(
            first.call(|_| Ok(())).await,
            Err(StorageError::Closed)
        ));
        assert!(matches!(
            stores.read(&a, |_| Ok(())).await,
            Err(StorageError::Closed)
        ));

        // After a restart, a cold read finds every database as it was left.
        let reopened = ConversationStores::open(directory, MAX_WRITERS, Arc::new(schema::NoBlobs))
            .await
            .unwrap();
        for (conversation, state) in [(&a, "a2"), (&b, "b"), (&c, "c")] {
            assert_eq!(
                reopened.read(conversation, root_state).await.unwrap(),
                Some(Some(state.to_owned()))
            );
        }
        assert_eq!(reopened.open_writers(), 0);
    }

    #[tokio::test]
    async fn first_calls_at_once_open_one_writer_and_both_commit() {
        let data = tempfile::tempdir().unwrap();
        let stores = ConversationStores::open(data.path().join("conversations"), MAX_WRITERS, Arc::new(schema::NoBlobs))
            .await
            .unwrap();
        let a = conversation(1);
        let (one, other) = (stores.db(&a), stores.db(&a));
        let (first, second) = tokio::join!(
            write_root(&one, &a, "first"),
            write_root(&other, &a, "second")
        );
        first.unwrap();
        second.unwrap();
        assert_eq!(stores.open_writers(), 1);
    }
}
