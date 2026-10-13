//! The server log's database: its schema, the writer thread that inserts
//! and trims, and the reads.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use demi_shared_types::{RequestId, Timestamp};
use demi_web_api_protocol::diagnostics::{LogLevel, ServerLogLine};
use rusqlite::{Connection, OpenFlags, OptionalExtension as _, params};

use super::{Message, Row};
use crate::StorageError;
use crate::schema::version_of;
use crate::sqlite;

const SCHEMA: &str = "
CREATE TABLE events (
    id INTEGER PRIMARY KEY,
    at INTEGER NOT NULL,
    level INTEGER NOT NULL,
    target TEXT NOT NULL,
    message TEXT NOT NULL,
    request_id TEXT,
    user_id TEXT,
    conversation_id TEXT,
    turn_id TEXT,
    fields TEXT NOT NULL
);
CREATE INDEX events_request ON events (request_id) WHERE request_id IS NOT NULL;
CREATE INDEX events_conversation ON events (conversation_id, at) WHERE conversation_id IS NOT NULL;
CREATE INDEX events_at ON events (at);
";

/// The most often the writer commits.
const BATCH_INTERVAL: Duration = Duration::from_millis(200);
/// How often the writer trims the log.
const TRIM_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// How long a line is kept.
const KEPT_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// The most the lines may take of the database.
const MOST_BYTES: i64 = 512 * 1024 * 1024;

/// What a read keeps.
#[derive(Debug, Clone, Default)]
pub struct LogQuery {
    pub request: Option<RequestId>,
    /// The `next` of an earlier read: lines after it, oldest first. Without
    /// it, the newest lines.
    pub since: Option<u64>,
    pub limit: u64,
    /// Lines at this level and above.
    pub level: Option<LogLevel>,
}

/// Lines oldest first, and the cursor the next read continues from.
#[derive(Debug, Clone, PartialEq)]
pub struct LogRead {
    pub lines: Vec<ServerLogLine>,
    pub next: u64,
}

fn level_number(level: LogLevel) -> i64 {
    match level {
        LogLevel::Trace => 0,
        LogLevel::Debug => 1,
        LogLevel::Info => 2,
        LogLevel::Warn => 3,
        LogLevel::Error => 4,
    }
}

fn level_of(number: i64) -> Option<LogLevel> {
    Some(match number {
        0 => LogLevel::Trace,
        1 => LogLevel::Debug,
        2 => LogLevel::Info,
        3 => LogLevel::Warn,
        4 => LogLevel::Error,
        _ => return None,
    })
}

pub(super) fn now_ms() -> i64 {
    // A clock before 1970 has no meaningful time to record.
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
}

/// The writer thread: it commits what the queue holds at most every
/// [`BATCH_INTERVAL`], first a line saying how many events the queue had no
/// room for, and trims the log as it starts and every [`TRIM_INTERVAL`].
pub(super) fn serve(path: &Path, queue: &Receiver<Message>, dropped: &AtomicU64) {
    let mut database = Database::open(path);
    let mut trim_at = Instant::now();
    let mut committed_at: Option<Instant> = None;
    let mut batch = Vec::new();
    loop {
        if Instant::now() >= trim_at {
            database.trim();
            trim_at = Instant::now() + TRIM_INTERVAL;
        }
        let mut closing = match queue.recv_timeout(trim_at.saturating_duration_since(Instant::now())) {
            Ok(Message::Row(row)) => {
                batch.push(row);
                false
            }
            Ok(Message::Close) | Err(RecvTimeoutError::Disconnected) => true,
            Err(RecvTimeoutError::Timeout) => continue,
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
                Message::Row(row) => batch.push(row),
                Message::Close => closing = true,
            }
        }
        let missed = dropped.swap(0, Ordering::Relaxed);
        if missed > 0 {
            batch.insert(0, dropped_line(missed));
        }
        database.insert(&batch);
        batch.clear();
        committed_at = Some(Instant::now());
        if closing {
            return;
        }
    }
}

/// The line that says how many events the queue had no room for.
fn dropped_line(missed: u64) -> Row {
    Row {
        at: now_ms(),
        level: LogLevel::Warn,
        target: module_path!().into(),
        message: format!("{missed} log events were dropped: the server log fell behind"),
        request_id: None,
        user_id: None,
        conversation_id: None,
        turn_id: None,
        fields: serde_json::Map::new(),
    }
}

/// The writer's connection, created again when it fails. Standard error,
/// which keeps every line, says why once, not once per lost line.
struct Database<'a> {
    path: &'a Path,
    connection: Option<Connection>,
    failing: bool,
}

impl<'a> Database<'a> {
    fn open(path: &'a Path) -> Self {
        let mut database = Self {
            path,
            connection: None,
            failing: false,
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
            Ok(connection) => Some(connection),
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

    /// Writes `rows` in one transaction. A database that refuses them is
    /// created again for the next ones; these are lost.
    fn insert(&mut self, rows: &[Row]) {
        if rows.is_empty() {
            return;
        }
        if self.connection.is_none() {
            self.connection = self.connect();
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

    /// Deletes the lines older than [`KEPT_FOR`], then the oldest until the
    /// lines take at most [`MOST_BYTES`], and returns the freed pages to the
    /// disk.
    fn trim(&mut self) {
        let Some(connection) = self.connection.as_mut() else {
            return;
        };
        if let Err(error) = trim(connection) {
            self.failed("could not be trimmed", &error);
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
            "INSERT INTO events (at, level, target, message, request_id, user_id, conversation_id, turn_id, fields)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        for row in rows {
            statement.execute(params![
                row.at,
                level_number(row.level),
                row.target,
                row.message,
                row.request_id,
                row.user_id,
                row.conversation_id,
                row.turn_id,
                serde_json::Value::Object(row.fields.clone()).to_string(),
            ])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

fn trim(connection: &mut Connection) -> Result<(), StorageError> {
    let oldest_kept = now_ms() - i64::try_from(KEPT_FOR.as_millis()).expect("a week fits i64");
    connection.execute("DELETE FROM events WHERE at < ?1", [oldest_kept])?;
    loop {
        let used: i64 = connection.query_row(
            "SELECT (page_count - freelist_count) * page_size
             FROM pragma_page_count(), pragma_freelist_count(), pragma_page_size()",
            [],
            |row| row.get(0),
        )?;
        if used <= MOST_BYTES {
            break;
        }
        // A tenth of the lines at a time, oldest first.
        let deleted = connection.execute(
            "DELETE FROM events WHERE id <= (
                 SELECT id FROM events ORDER BY id
                 LIMIT 1 OFFSET (SELECT count(*) / 10 FROM events)
             )",
            [],
        )?;
        if deleted == 0 {
            break;
        }
    }
    connection.execute_batch("PRAGMA incremental_vacuum")?;
    Ok(())
}

/// The lines `query` asks for, from a connection of its own: the writer's
/// next commit does not wait for it.
pub(super) fn read(path: &Path, query: &LogQuery) -> Result<LogRead, StorageError> {
    let connection = match Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(connection) => connection,
        // Not written yet, or being created again.
        Err(rusqlite::Error::SqliteFailure(error, _))
            if error.code == rusqlite::ErrorCode::CannotOpen =>
        {
            return Ok(LogRead {
                lines: Vec::new(),
                next: query.since.unwrap_or(0),
            });
        }
        Err(error) => return Err(error.into()),
    };
    connection.busy_timeout(sqlite::BUSY_TIMEOUT)?;
    let newest: Option<i64> = connection
        .query_row("SELECT max(id) FROM events", [], |row| row.get(0))
        .optional()?
        .flatten();
    let newest = newest.map_or(0, |id| u64::try_from(id).unwrap_or(0));
    let request = query.request.as_ref().map(RequestId::as_str);
    let level = query.level.map(level_number);
    let limit = i64::try_from(query.limit).unwrap_or(i64::MAX);
    let filters = "(?1 IS NULL OR request_id = ?1) AND (?2 IS NULL OR level >= ?2)";
    let columns = "id, at, level, target, message, request_id, user_id, conversation_id, turn_id, fields";
    let (mut lines, more) = match query.since {
        // A cursor ahead of the newest line comes from a log created again:
        // it continues from the oldest line.
        Some(since) => {
            let since = if since > newest { 0 } else { since };
            let mut statement = connection.prepare(&format!(
                "SELECT {columns} FROM events WHERE id > ?3 AND {filters} ORDER BY id LIMIT ?4"
            ))?;
            let mut lines = statement
                .query_map(
                    params![request, level, i64::try_from(since).unwrap_or(i64::MAX), limit + 1],
                    line_of,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            let more = lines.len() as u64 > query.limit;
            lines.truncate(usize::try_from(query.limit).unwrap_or(usize::MAX));
            (lines, more)
        }
        None => {
            let mut statement = connection.prepare(&format!(
                "SELECT {columns} FROM events WHERE {filters} ORDER BY id DESC LIMIT ?3"
            ))?;
            let mut lines = statement
                .query_map(params![request, level, limit], line_of)?
                .collect::<Result<Vec<_>, _>>()?;
            lines.reverse();
            (lines, false)
        }
    };
    // With more to come, the next read continues after the last line
    // returned; otherwise after everything this one looked at.
    let next = match lines.last() {
        Some((id, _)) if more => *id,
        _ => newest,
    };
    Ok(LogRead {
        lines: lines.drain(..).map(|(_, line)| line).collect(),
        next,
    })
}

/// A row as a line, its ID columns among its fields again.
fn line_of(row: &rusqlite::Row<'_>) -> rusqlite::Result<(u64, ServerLogLine)> {
    let corrupt = |index, error: Box<dyn std::error::Error + Send + Sync>| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, error)
    };
    let id: i64 = row.get(0)?;
    let at: i64 = row.get(1)?;
    let level: i64 = row.get(2)?;
    let fields: String = row.get(9)?;
    let mut fields: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&fields).map_err(|error| corrupt(9, error.into()))?;
    for (index, name) in [(5, "request_id"), (6, "user_id"), (7, "conversation_id"), (8, "turn_id")] {
        if let Some(value) = row.get::<_, Option<String>>(index)? {
            fields.insert(name.into(), value.into());
        }
    }
    Ok((
        u64::try_from(id).unwrap_or(0),
        ServerLogLine {
            at: Timestamp::from_millisecond(at).map_err(|error| corrupt(1, error.into()))?,
            level: level_of(level).ok_or_else(|| corrupt(2, format!("no level {level}").into()))?,
            target: row.get(3)?,
            message: row.get(4)?,
            fields,
        },
    ))
}
