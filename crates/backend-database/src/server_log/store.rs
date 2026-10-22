//! The log's database: its schema and the writer thread that inserts and
//! trims.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use demi_shared_types::{LogLevel, LogLine, LogOrigin};
use rusqlite::{Connection, OpenFlags, OptionalExtension as _, params};

use super::{Limits, Message, Row};
use crate::StorageError;
use crate::schema::version_of;
use crate::sqlite;

const SCHEMA: &str = "
CREATE TABLE events (
    id INTEGER PRIMARY KEY,
    at INTEGER NOT NULL,
    origin TEXT NOT NULL,
    level TEXT NOT NULL,
    event TEXT NOT NULL,
    request_id TEXT,
    user_id TEXT,
    conversation_id TEXT,
    turn_id TEXT,
    host_id TEXT,
    page_id TEXT,
    seq INTEGER,
    fields TEXT NOT NULL
);
CREATE INDEX events_request ON events (request_id) WHERE request_id IS NOT NULL;
CREATE INDEX events_conversation ON events (conversation_id, at) WHERE conversation_id IS NOT NULL;
CREATE INDEX events_host ON events (host_id, at) WHERE host_id IS NOT NULL;
CREATE INDEX events_host_seq ON events (host_id, seq) WHERE seq IS NOT NULL;
CREATE INDEX events_page ON events (page_id, at) WHERE page_id IS NOT NULL;
CREATE INDEX events_at ON events (at);
";

/// The most often the writer commits.
const BATCH_INTERVAL: Duration = Duration::from_millis(200);
/// How often the writer trims the log.
const TRIM_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// The most lines one deletion of a trim removes, so that the writer is
/// never held for more than a fraction of a second.
const TRIM_BATCH: i64 = 10_000;
/// The most pages one step of returning the freed pages to the disk frees.
const VACUUM_PAGES: i64 = 2_000;

fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Trace => "trace",
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    }
}

fn origin_name(origin: LogOrigin) -> &'static str {
    match origin {
        LogOrigin::Backend => "backend",
        LogOrigin::Runner => "runner",
        LogOrigin::Page => "page",
    }
}

pub(super) fn now_ms() -> i64 {
    // A clock before 1970 has no meaningful time to record.
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
}

/// A line the log writes about itself.
fn own_line(level: LogLevel, event: &str, fields: serde_json::Value) -> Row {
    let mut line = LogLine::new(now_ms(), LogOrigin::Backend, level, event);
    if let serde_json::Value::Object(fields) = fields {
        line.fields = fields;
    }
    Row { line, seq: None }
}

/// The writer thread: it commits what the queue holds at most every
/// [`BATCH_INTERVAL`], first a line saying how many events the queue had no
/// room for, and trims the log as it starts and every [`TRIM_INTERVAL`], one
/// batch between two commits.
pub(super) fn serve(path: &Path, limits: Limits, queue: &Receiver<Message>, dropped: &AtomicU64, waiting: &AtomicU64) {
    let mut database = Database::open(path);
    let mut trim_at = Instant::now();
    let mut trimming: Option<Trim> = None;
    let mut committed_at: Option<Instant> = None;
    let mut batch = Vec::new();
    loop {
        if trimming.is_none() && Instant::now() >= trim_at {
            trimming = Some(Trim::start());
        }
        let wait = if trimming.is_some() {
            Duration::ZERO
        } else {
            trim_at.saturating_duration_since(Instant::now())
        };
        let mut closing = match queue.recv_timeout(wait) {
            Ok(Message::Row(row)) => {
                waiting.fetch_sub(1, Ordering::Relaxed);
                batch.push(row);
                false
            }
            Ok(Message::Close) | Err(RecvTimeoutError::Disconnected) => true,
            Err(RecvTimeoutError::Timeout) => {
                // Nothing waits: one batch of the trim.
                if let Some(trim) = trimming.as_mut()
                    && let Some(line) = database.trim_step(trim, limits)
                {
                    trimming = None;
                    trim_at = Instant::now() + TRIM_INTERVAL;
                    if let Some(line) = line {
                        batch.push(line);
                    }
                }
                if batch.is_empty() {
                    continue;
                }
                false
            }
        };
        // What else arrives before the interval since the last commit ends
        // goes into the same transaction.
        let due = committed_at.map_or_else(Instant::now, |at| at + BATCH_INTERVAL);
        while !closing && batch.len() < super::QUEUE {
            let message = match due.checked_duration_since(Instant::now()) {
                Some(left) if !left.is_zero() => match queue.recv_timeout(left) {
                    Ok(message) => message,
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => Message::Close,
                },
                _ => match queue.try_recv() {
                    Ok(message) => message,
                    Err(_) => break,
                },
            };
            match message {
                Message::Row(row) => {
                    waiting.fetch_sub(1, Ordering::Relaxed);
                    batch.push(row);
                }
                Message::Close => closing = true,
            }
        }
        let missed = dropped.swap(0, Ordering::Relaxed);
        if missed > 0 {
            batch.insert(
                0,
                own_line(
                    LogLevel::Warn,
                    "log.dropped",
                    serde_json::json!({ "dropped": missed, "why": "the writer fell behind" }),
                ),
            );
        }
        database.insert(&mut batch);
        batch.clear();
        committed_at = Some(Instant::now());
        if closing {
            return;
        }
    }
}

/// One trim, as the batches it takes.
struct Trim {
    started: Instant,
    deleted: u64,
    batches: u64,
}

impl Trim {
    fn start() -> Self {
        Self {
            started: Instant::now(),
            deleted: 0,
            batches: 0,
        }
    }
}

/// The writer's connection, created again when it fails. Standard error,
/// which keeps every line, says why once, not once per lost line.
struct Database<'a> {
    path: &'a Path,
    connection: Option<Connection>,
    failing: bool,
    /// Lines about the log itself, written with the next commit.
    own: Vec<Row>,
}

impl<'a> Database<'a> {
    fn open(path: &'a Path) -> Self {
        let mut database = Self {
            path,
            connection: None,
            failing: false,
            own: Vec::new(),
        };
        database.connection = database.connect();
        database
    }

    /// The database at `path`, of this schema: one of another schema, or one
    /// that does not open, is removed and created again.
    fn connect(&mut self) -> Option<Connection> {
        match open_current(self.path) {
            Ok(connection) => return Some(connection),
            Err(error) => self.failed("could not be opened; it is created again", &error),
        }
        remove(self.path);
        match open_current(self.path) {
            Ok(connection) => {
                self.own.push(own_line(
                    LogLevel::Warn,
                    "log.recreated",
                    serde_json::json!({ "why": "the database could not be opened or was of another schema" }),
                ));
                Some(connection)
            }
            Err(error) => {
                self.failed("could not be created", &error);
                None
            }
        }
    }

    fn failed(&mut self, what: &str, error: &StorageError) {
        if !self.failing {
            eprintln!("demi-backend: the server log {what}: {error}");
        }
        self.failing = true;
    }

    /// Writes `rows`, with the lines about the log itself, in one
    /// transaction. A database that refuses them is created again for the
    /// next ones; these are lost.
    fn insert(&mut self, rows: &mut Vec<Row>) {
        if self.connection.is_none() {
            self.connection = self.connect();
        }
        if !self.own.is_empty() {
            rows.splice(0..0, self.own.drain(..));
        }
        if rows.is_empty() {
            return;
        }
        let Some(connection) = self.connection.as_mut() else {
            return;
        };
        match insert(connection, rows) {
            Ok(()) => self.failing = false,
            Err(error) => {
                self.failed("could not be written; it is created again", &error);
                self.connection = None;
                remove(self.path);
            }
        }
    }

    /// One batch of a trim: deletes up to [`TRIM_BATCH`] lines older than
    /// [`Limits::keep`], else the oldest while the lines take more than
    /// [`Limits::max_bytes`], else returns [`VACUUM_PAGES`] freed pages to
    /// the disk. Answers `None` while more is left; at its end, the line
    /// that tells what the trim did, when it deleted anything.
    fn trim_step(&mut self, trim: &mut Trim, limits: Limits) -> Option<Option<Row>> {
        let Some(connection) = self.connection.as_mut() else {
            return Some(None);
        };
        match trim_batch(connection, limits) {
            Ok(Step::Deleted(count)) => {
                trim.deleted += count;
                trim.batches += 1;
                None
            }
            Ok(Step::Vacuumed) => None,
            Ok(Step::Done) => Some((trim.deleted > 0).then(|| {
                own_line(
                    LogLevel::Info,
                    "log.trimmed",
                    serde_json::json!({
                        "deleted": trim.deleted,
                        "batches": trim.batches,
                        "ms": u64::try_from(trim.started.elapsed().as_millis()).unwrap_or(u64::MAX),
                    }),
                )
            })),
            Err(error) => {
                self.failed("could not be trimmed", &error);
                Some(None)
            }
        }
    }
}

fn open_current(path: &Path) -> Result<Connection, StorageError> {
    let connection = Connection::open(path)?;
    // Before any table: a database created with it returns the pages its
    // deletes free to the disk when asked.
    connection.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
    sqlite::configure(&connection)?;
    let version = version_of(SCHEMA);
    let recorded: i32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if recorded == version {
        return Ok(connection);
    }
    let empty: bool = connection.query_row(
        "SELECT NOT EXISTS (SELECT 1 FROM sqlite_schema)",
        [],
        |row| row.get(0),
    )?;
    if !empty {
        return Err(StorageError::OtherSchema {
            path: path.to_owned(),
        });
    }
    connection.execute_batch(SCHEMA)?;
    connection.pragma_update(None, "user_version", version)?;
    Ok(connection)
}

/// Removes the database and its journal files; one already gone is what
/// was asked.
fn remove(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let mut file = path.as_os_str().to_owned();
        file.push(suffix);
        if let Err(error) = std::fs::remove_file(&file)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!(
                "demi-backend: the server log's {} could not be removed: {error}",
                Path::new(&file).display()
            );
        }
    }
}

fn insert(connection: &mut Connection, rows: &[Row]) -> Result<(), StorageError> {
    let transaction = connection.transaction()?;
    {
        let mut statement = transaction.prepare_cached(
            "INSERT INTO events (at, origin, level, event, request_id, user_id, conversation_id, turn_id, host_id, page_id, seq, fields)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )?;
        for Row { line, seq } in rows {
            statement.execute(params![
                line.at,
                origin_name(line.origin),
                level_name(line.level),
                line.event,
                line.request_id,
                line.user_id,
                line.conversation_id,
                line.turn_id,
                line.host_id,
                line.page_id,
                seq.map(|seq| i64::try_from(seq).unwrap_or(i64::MAX)),
                serde_json::Value::Object(line.fields.clone()).to_string(),
            ])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

enum Step {
    /// This many lines were deleted; more may be left.
    Deleted(u64),
    /// Freed pages went back to the disk; more may be left.
    Vacuumed,
    Done,
}

fn trim_batch(connection: &mut Connection, limits: Limits) -> Result<Step, StorageError> {
    let oldest_kept = now_ms() - i64::try_from(limits.keep.as_millis()).unwrap_or(i64::MAX);
    let aged = connection.execute(
        "DELETE FROM events WHERE id IN (SELECT id FROM events WHERE at < ?1 LIMIT ?2)",
        params![oldest_kept, TRIM_BATCH],
    )?;
    if aged > 0 {
        return Ok(Step::Deleted(aged as u64));
    }
    let used: i64 = connection.query_row(
        "SELECT (page_count - freelist_count) * page_size
         FROM pragma_page_count(), pragma_freelist_count(), pragma_page_size()",
        [],
        |row| row.get(0),
    )?;
    if used > i64::try_from(limits.max_bytes).unwrap_or(i64::MAX) {
        // The oldest lines first.
        let deleted = connection.execute(
            "DELETE FROM events WHERE id IN (SELECT id FROM events ORDER BY id LIMIT ?1)",
            [TRIM_BATCH],
        )?;
        if deleted > 0 {
            return Ok(Step::Deleted(deleted as u64));
        }
    }
    let free: i64 = connection.pragma_query_value(None, "freelist_count", |row| row.get(0))?;
    if free > 0 {
        connection.execute_batch(&format!("PRAGMA incremental_vacuum({VACUUM_PAGES})"))?;
        return Ok(Step::Vacuumed);
    }
    Ok(Step::Done)
}

/// The highest `seq` of the Host's lines, from a connection of its own: the
/// writer's next commit does not wait for it.
pub(super) fn last_seq(path: &Path, host_id: &str) -> Result<Option<u64>, StorageError> {
    let connection = match Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(connection) => connection,
        // Not written yet, or being created again.
        Err(rusqlite::Error::SqliteFailure(error, _))
            if error.code == rusqlite::ErrorCode::CannotOpen =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    };
    connection.busy_timeout(sqlite::BUSY_TIMEOUT)?;
    let seq: Option<i64> = connection
        .query_row(
            "SELECT max(seq) FROM events WHERE host_id = ?1 AND seq IS NOT NULL",
            [host_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    Ok(seq.and_then(|seq| u64::try_from(seq).ok()))
}
