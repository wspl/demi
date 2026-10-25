//! The conversion of a conversation's database of the last schema whose rows
//! held whole blocks ([`schema::LEGACY_CONVERSATION`]) into the current one,
//! whose rows are records that name their contents (`records.rs`).
//!
//! ```text
//! <id>.sqlite             the old file, read and never written
//!      |  stream its rows; every core parses, splits, checks and
//!      |  compresses them; one thread writes them in order, a few
//!      |  hundred to a transaction
//!      v
//! <id>.sqlite.converting  the new file
//!      |  rename, once every row is written
//!      v
//! <id>.sqlite             the new file
//! ```
//!
//! Each row is checked before it is written: its record joined with the
//! contents it names, held in memory, reads as the block it came from, and
//! its compressed body decompresses to the record. The bytes in flight are
//! bounded by a few rows per core.
//!
//! The old file stays as it was until the rename replaces it, so an upgrade's
//! snapshot holds it by a hard link, and a conversion that stops at any step
//! starts over from it. Every row, every node and every path stay as they
//! were: only how a row is stored changes.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path as FilePath, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use demi_shared_types::Block;
use rusqlite::{Connection, OpenFlags, OptionalExtension};

use super::columns::json;
use super::contents::{self, Hash, LEVEL};
use super::records::{self, Codec, Prepared, PreparedContent};
use super::schema::{self, LegacyForm, ReadBlob};
use super::{StorageError, sqlite};

/// How many rows one transaction of the new file writes: few enough that
/// no transaction holds much, many enough that commits cost little.
const ROWS_PER_TRANSACTION: usize = 500;

/// How many rows the dictionary is trained on, spread over the file.
const SAMPLES: i64 = 3000;

/// How many rows wait between the conversion's threads, per core: enough
/// that no core waits for work.
const IN_FLIGHT: usize = 16;

/// How many bytes of old rows the conversion holds between reading them and
/// writing them, as each is held a few times over while it is prepared:
/// a file of large rows, or one large row the writer waits for, would hold
/// gigabytes without it. A row larger than this goes through alone.
const HELD_BYTES: usize = 64 << 20;

/// The tables copied as they are, in an order their foreign keys accept,
/// before the rows.
const COPIED: [&str; 10] = [
    "nodes",
    "side_questions",
    "sequences",
    "path_runs",
    "share_entries",
    "share_blobs",
    "share_attachments",
    "command_outputs",
    "running_commands",
    "attachments",
];

/// The file a conversion writes before it renames it over `path`.
pub(crate) fn converting(path: &FilePath) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".converting");
    PathBuf::from(name)
}

/// Converts the database at `path` when its rows hold whole blocks: as it
/// is when it has a form this build converts ([`schema::legacy_form`]), else
/// after migrating it to [`schema::LEGACY_CONVERSATION`]. The caller holds
/// the file's opening turn, and no connection has it open. `progress` hears
/// the share of the rows done, from 0 to 1, as they are written.
pub(crate) fn convert_if_due(path: &FilePath, blobs: &dyn ReadBlob, progress: &(dyn Fn(f64) + Sync)) -> Result<(), StorageError> {
    if !path.try_exists()? {
        return Ok(());
    }
    let form;
    {
        let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        let recorded: i32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if schema::CONVERSATION.current(recorded).is_some() || schema::LEGACY_CONVERSATION.current(recorded).is_none() {
            return Ok(());
        }
        sqlite::configure(&connection)?;
        form = match schema::legacy_form(recorded) {
            Some(form) => form,
            None => {
                schema::LEGACY_CONVERSATION.apply(&mut connection, path, blobs)?;
                LegacyForm::Current
            }
        };
        // The log's frames go into the file, so the file alone holds the
        // database and no log beside it outlives the rename.
        connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    }
    let database = path.file_name().map(|name| name.to_string_lossy().into_owned());
    tracing::info!(event = "conversation.conversion_started", database, "a conversation's database converts");
    let started = std::time::Instant::now();
    let converted = convert(path, form, progress);
    if converted.is_err() {
        // The new file is not the database yet: it goes, so a conversion
        // that failed, as on a full disk, leaves no space taken.
        if let Err(error) = remove_with_logs(&converting(path)) {
            tracing::warn!(
                event = "conversation.conversion_left",
                database,
                error = &error as &dyn std::error::Error,
                "the file of a conversion that failed stays beside the database"
            );
        }
    }
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match &converted {
        Ok(rows) => tracing::info!(event = "conversation.conversion_ended", database, rows, ms, "a conversation's database converted"),
        Err(error) => tracing::warn!(
            event = "conversation.conversion_failed",
            database,
            ms,
            error = error as &dyn std::error::Error,
            "a conversation's database did not convert and stays as it was"
        ),
    }
    converted.map(|_| ())
}

/// Writes the new file beside `path`, checks it and renames it over
/// `path`; answers how many rows it converted.
fn convert(path: &FilePath, form: LegacyForm, progress: &(dyn Fn(f64) + Sync)) -> Result<u64, StorageError> {
    let target = converting(path);
    remove_with_logs(&target)?;
    let legacy = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    let mut new = Connection::open(&target)?;
    schema::CONVERSATION.prepare_new(&new)?;
    // The new file is not the database until the rename, and a conversion
    // that stops starts over: it needs no journal until then.
    new.pragma_update(None, "journal_mode", "OFF")?;
    new.pragma_update(None, "synchronous", "OFF")?;
    new.pragma_update(None, "foreign_keys", true)?;
    schema::CONVERSATION.apply(&mut new, &target, &schema::NoBlobs)?;
    new.execute("ATTACH DATABASE ?1 AS legacy", [path.to_string_lossy()])?;
    let dictionary = dictionary(&legacy, form)?;
    let dictionary = {
        let transaction = new.transaction()?;
        for table in COPIED {
            // By the legacy table's columns: a column the new one added
            // since takes its default.
            let columns = legacy_columns(&transaction, table)?.join(", ");
            transaction.execute(&format!("INSERT INTO main.{table} ({columns}) SELECT {columns} FROM legacy.{table}"), [])?;
        }
        if form == LegacyForm::AutomationNames {
            schema::name_schedules(&transaction, "nodes", "state")?;
        }
        let dictionary = match dictionary {
            Some(bytes) => {
                let id: i64 = transaction.query_row("INSERT INTO dictionaries (bytes) VALUES (?1) RETURNING id", [&bytes], |row| row.get(0))?;
                Some((id, bytes))
            }
            None => None,
        };
        transaction.commit()?;
        dictionary
    };
    let reader = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    let rows = convert_rows(reader, &mut new, form, dictionary.as_ref(), progress)?;
    {
        let transaction = new.transaction()?;
        for table in ["block_blobs", "block_attachments"] {
            transaction.execute(&format!("INSERT INTO main.{table} SELECT * FROM legacy.{table}"), [])?;
        }
        convert_copies(&legacy, &transaction, form)?;
        transaction.commit()?;
    }
    new.execute("DETACH DATABASE legacy", [])?;
    drop(legacy);
    let check: String = new.query_row("PRAGMA foreign_key_check", [], |row| row.get(0)).optional()?.unwrap_or_default();
    if !check.is_empty() {
        return Err(StorageError::Corrupt {
            table: "blocks",
            column: "node_id",
            reason: format!("the converted {check} names a row it does not hold"),
        });
    }
    let mode: String = new.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StorageError::JournalMode(mode));
    }
    new.close().map_err(|(_, error)| error)?;
    std::fs::File::open(&target)?.sync_all()?;
    for suffix in ["-wal", "-shm"] {
        remove_file(&with_suffix(path, suffix))?;
    }
    std::fs::rename(&target, path)?;
    if let Some(directory) = path.parent() {
        std::fs::File::open(directory)?.sync_all()?;
    }
    Ok(rows)
}

/// A dictionary trained on records of rows spread over the old file, each
/// row sampled once. The rows are read in order and split on every core;
/// only their records' bodies, a few kilobytes each, are kept.
fn dictionary(legacy: &Connection, form: LegacyForm) -> Result<Option<Vec<u8>>, StorageError> {
    let last: Option<i64> = legacy.query_row("SELECT max(rowid) FROM blocks", [], |row| row.get(0))?;
    let Some(last) = last.filter(|last| *last >= records::DICTIONARY_AFTER) else {
        return Ok(None);
    };
    let workers = workers();
    let samples = std::thread::scope(|scope| {
        let (texts_out, texts_in) = mpsc::sync_channel::<String>(IN_FLIGHT * workers);
        let texts_in = Arc::new(Mutex::new(texts_in));
        let splitting: Vec<_> = (0..workers)
            .map(|_| {
                let texts_in = texts_in.clone();
                scope.spawn(move || {
                    let mut bodies = Vec::new();
                    loop {
                        let text = texts_in.lock().unwrap_or_else(PoisonError::into_inner).recv();
                        let Ok(text) = text else {
                            return Ok::<_, StorageError>(bodies);
                        };
                        bodies.push(records::split(&legacy_block("blocks", &text, form)?)?.body);
                    }
                })
            })
            .collect();
        drop(texts_in);
        let read = (|| {
            let mut statement = legacy.prepare("SELECT rowid, block FROM blocks WHERE rowid >= ?1 ORDER BY rowid LIMIT 1")?;
            let mut previous = None;
            for at in 0..SAMPLES {
                let row: Option<(i64, String)> =
                    statement.query_row([last * at / SAMPLES], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
                let Some((rowid, text)) = row else {
                    continue;
                };
                if previous == Some(rowid) {
                    continue;
                }
                previous = Some(rowid);
                if texts_out.send(text).is_err() {
                    break;
                }
            }
            Ok::<_, StorageError>(())
        })();
        drop(texts_out);
        let bodies = splitting
            .into_iter()
            .map(|part| part.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic)))
            .collect::<Result<Vec<_>, _>>()?;
        read?;
        Ok::<_, StorageError>(bodies)
    })?;
    Ok(records::train(&samples.concat()))
}

/// How many threads prepare rows: one per core the process may use.
fn workers() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}

/// The bytes of the rows read and not yet written, which the reader waits
/// on while they would pass [`HELD_BYTES`].
#[derive(Default)]
struct Window {
    held: Mutex<(usize, bool)>,
    changed: Condvar,
}

impl Window {
    /// Waits until a row of `bytes` fits, and answers false once the writer
    /// has stopped.
    fn admit(&self, bytes: usize) -> bool {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        while held.0 > 0 && held.0 + bytes > HELD_BYTES && !held.1 {
            held = self.changed.wait(held).unwrap_or_else(PoisonError::into_inner);
        }
        held.0 += bytes;
        !held.1
    }

    /// A row of `bytes` was written.
    fn release(&self, bytes: usize) {
        self.held.lock().unwrap_or_else(PoisonError::into_inner).0 -= bytes;
        self.changed.notify_all();
    }

    /// The writer stopped: the reader waits no more.
    fn close(&self) {
        self.held.lock().unwrap_or_else(PoisonError::into_inner).1 = true;
        self.changed.notify_all();
    }
}

/// Closes the window as the writer stops, however it stops.
struct Closing<'a>(&'a Window);

impl Drop for Closing<'_> {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// A row of the old file, numbered in the order it is written.
struct OldRow {
    at: u64,
    rowid: i64,
    node: String,
    idx: i64,
    follows: Option<i64>,
    replaces: Option<i64>,
    text: String,
}

/// A row ready to be written.
struct ReadyRow {
    at: u64,
    /// The old row's size, which the window holds until it is written.
    held: usize,
    rowid: i64,
    node: String,
    idx: i64,
    follows: Option<i64>,
    replaces: Option<i64>,
    record: Prepared,
}

/// The contents some row prepared so far: a later row that holds one again
/// leaves it uncompressed, since the writer finds it written.
#[derive(Default)]
struct Claims(Mutex<HashSet<Hash>>);

impl Claims {
    /// Whether this call is the first to claim `hash`.
    fn claim(&self, hash: Hash) -> bool {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).insert(hash)
    }
}

/// Writes a record for each row of the old file, and answers how many it
/// wrote.
///
/// ```text
/// reader ──rows──► one worker per core ──ready rows──► writer (this thread)
/// ```
///
/// The reader reads the rows in order, the workers prepare them in any
/// order, and the writer writes them in their order, so a row's index and
/// its place among its node's rows are the old file's. A window bounds the
/// bytes between the reader and the writer, the rows the writer holds while
/// it waits for an earlier one included.
fn convert_rows(
    legacy: Connection,
    new: &mut Connection,
    form: LegacyForm,
    dictionary: Option<&(i64, Vec<u8>)>,
    progress: &(dyn Fn(f64) + Sync),
) -> Result<u64, StorageError> {
    let last: i64 = legacy.query_row("SELECT coalesce(max(rowid), 0) FROM blocks", [], |row| row.get(0))?;
    let workers = workers();
    let claims = Claims::default();
    let window = Window::default();
    std::thread::scope(|scope| {
        let (rows_out, rows_in) = mpsc::sync_channel::<OldRow>(IN_FLIGHT * workers);
        let (ready_out, ready_in) = mpsc::sync_channel::<Result<ReadyRow, StorageError>>(IN_FLIGHT * workers);
        let window = &window;
        let reader = scope.spawn(move || read_rows(&legacy, &rows_out, window));
        // Each worker holds the rows' receiver, so once all have ended the
        // reader's next send fails and it ends too.
        let rows_in = Arc::new(Mutex::new(rows_in));
        for _ in 0..workers {
            let (rows_in, ready_out, claims) = (rows_in.clone(), ready_out.clone(), &claims);
            scope.spawn(move || prepare_rows(&rows_in, &ready_out, form, dictionary.map(|(_, bytes)| bytes.as_slice()), claims));
        }
        drop((rows_in, ready_out));
        let written = write_rows(new, ready_in, window, dictionary.map(|(id, _)| *id), last, progress);
        // A writer that stopped dropped what feeds it, so the workers and the
        // reader end; its failure is the one to answer.
        let read = reader.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        let written = written?;
        read?;
        Ok(written)
    })
}

/// Sends every row of the old file, in order, as the window takes it, until
/// the rows end or the writer stops.
fn read_rows(legacy: &Connection, out: &SyncSender<OldRow>, window: &Window) -> Result<(), StorageError> {
    let mut rows = legacy.prepare("SELECT rowid, node_id, idx, block, follows, replaces FROM blocks NOT INDEXED ORDER BY rowid")?;
    let mut rows = rows.query([])?;
    let mut at = 0;
    while let Some(row) = rows.next()? {
        let old = OldRow {
            at,
            rowid: row.get(0)?,
            node: row.get(1)?,
            idx: row.get(2)?,
            text: row.get(3)?,
            follows: row.get(4)?,
            replaces: row.get(5)?,
        };
        if !window.admit(old.text.len()) || out.send(old).is_err() {
            return Ok(());
        }
        at += 1;
    }
    Ok(())
}

/// Prepares the rows it takes until they end or the writer stops.
fn prepare_rows(
    rows: &Mutex<Receiver<OldRow>>,
    out: &SyncSender<Result<ReadyRow, StorageError>>,
    form: LegacyForm,
    dictionary: Option<&[u8]>,
    claims: &Claims,
) {
    let compressor = match dictionary {
        Some(bytes) => zstd::bulk::Compressor::with_dictionary(LEVEL, bytes),
        None => zstd::bulk::Compressor::new(LEVEL),
    };
    let decompressor = match dictionary {
        Some(bytes) => zstd::bulk::Decompressor::with_dictionary(bytes),
        None => zstd::bulk::Decompressor::new(),
    };
    let (mut compressor, mut decompressor) = match (compressor, decompressor) {
        (Ok(compressor), Ok(decompressor)) => (compressor, decompressor),
        (Err(error), _) | (_, Err(error)) => {
            let _ = out.send(Err(error.into()));
            return;
        }
    };
    loop {
        let row = rows.lock().unwrap_or_else(PoisonError::into_inner).recv();
        let Ok(row) = row else {
            return;
        };
        let ready = prepare(&row.text, form, &mut compressor, &mut decompressor, claims).map(|record| ReadyRow {
            at: row.at,
            held: row.text.len(),
            rowid: row.rowid,
            node: row.node,
            idx: row.idx,
            follows: row.follows,
            replaces: row.replaces,
            record,
        });
        if out.send(ready).is_err() {
            return;
        }
    }
}

/// The record of the old row `text`, checked: joined with its contents it
/// reads as the block, and its compressed body decompresses to it.
fn prepare(
    text: &str,
    form: LegacyForm,
    compressor: &mut zstd::bulk::Compressor<'static>,
    decompressor: &mut zstd::bulk::Decompressor<'static>,
    claims: &Claims,
) -> Result<Prepared, StorageError> {
    let block = legacy_block("blocks", text, form)?;
    let document = records::document(&block);
    let split = records::split_document(document.clone())?;
    let mut joined: serde_json::Value =
        serde_json::from_slice(&split.body).map_err(|error| differs(format!("its record is not JSON: {error}")))?;
    records::join_with(&mut joined, &mut |hash| {
        split
            .contents
            .iter()
            .find(|(held, _)| held == hash)
            .map(|(_, bytes)| bytes.clone())
            .ok_or_else(|| differs(format!("its record names the content {}, which it does not hold", hash.hex())))
    })?;
    if joined != document {
        return Err(differs("its record does not read back as the block".into()));
    }
    let body = compressor.compress(&split.body)?;
    if contents::decompress(&body, Some(decompressor))? != split.body {
        return Err(differs("its compressed record does not decompress to it".into()));
    }
    let contents = split
        .contents
        .into_iter()
        .map(|(hash, bytes)| {
            Ok((
                hash,
                if claims.claim(hash) {
                    PreparedContent::Compressed(zstd::bulk::compress(&bytes, LEVEL)?)
                } else {
                    PreparedContent::Claimed(bytes)
                },
            ))
        })
        .collect::<Result<_, StorageError>>()?;
    Ok(Prepared {
        columns: split.columns,
        body,
        contents,
    })
}

/// Writes the ready rows in their order, a few hundred to a transaction,
/// and answers how many it wrote.
fn write_rows(
    new: &mut Connection,
    ready: Receiver<Result<ReadyRow, StorageError>>,
    window: &Window,
    dictionary: Option<i64>,
    last: i64,
    progress: &(dyn Fn(f64) + Sync),
) -> Result<u64, StorageError> {
    let _closing = Closing(window);
    let mut waiting = BTreeMap::new();
    let mut next = 0u64;
    let mut transaction = new.transaction()?;
    let mut batch = 0;
    for row in ready {
        let row = row?;
        waiting.insert(row.at, row);
        while let Some(row) = waiting.remove(&next) {
            records::insert_prepared(&transaction, &row.node, row.idx, row.follows, row.replaces, dictionary, row.record)?;
            window.release(row.held);
            next += 1;
            batch += 1;
            if batch == ROWS_PER_TRANSACTION {
                transaction.commit()?;
                transaction = new.transaction()?;
                batch = 0;
                if last > 0 {
                    progress(row.rowid as f64 / last as f64);
                }
            }
        }
    }
    if !waiting.is_empty() {
        return Err(differs("a row was not prepared".into()));
    }
    transaction.commit()?;
    progress(1.0);
    Ok(next)
}

/// A row whose record would not read back as it was.
fn differs(reason: String) -> StorageError {
    StorageError::Corrupt {
        table: "blocks",
        column: "body",
        reason,
    }
}

/// Writes a record for each of the shares' copies of the old file.
fn convert_copies(legacy: &Connection, transaction: &rusqlite::Transaction<'_>, form: LegacyForm) -> Result<(), StorageError> {
    let codec = Codec::new(transaction);
    let mut copies = legacy.prepare("SELECT token, idx, block FROM share_blocks ORDER BY token, idx")?;
    let mut copies = copies.query([])?;
    while let Some(copy) = copies.next()? {
        let (token, idx, text): (String, i64, String) = (copy.get(0)?, copy.get(1)?, copy.get(2)?);
        let block = legacy_block("share_blocks", &text, form)?;
        records::insert_copy(transaction, &codec, &token, idx, &block)?;
    }
    Ok(())
}

/// The block a row of the old file's `table` holds as `text`, in the
/// current format.
fn legacy_block(table: &'static str, text: &str, form: LegacyForm) -> Result<Block, StorageError> {
    if form == LegacyForm::AutomationNames && text.contains("automation") {
        let corrupt = |reason: String| StorageError::Corrupt { table, column: "block", reason };
        let mut value: serde_json::Value = serde_json::from_str(text).map_err(|error| corrupt(error.to_string()))?;
        schema::named_marks(table, &mut value);
        return demi_shared_types::decode_value::<Block>(value).map_err(|error| corrupt(error.to_string()));
    }
    json(table, "block", text)
}

fn with_suffix(path: &FilePath, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn remove_file(path: &FilePath) -> Result<(), StorageError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Removes the database at `path` with the journal and logs beside it.
pub(crate) fn remove_with_logs(path: &FilePath) -> Result<(), StorageError> {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        remove_file(&with_suffix(path, suffix))?;
    }
    Ok(())
}

/// The columns of the legacy database's `table`, in order.
fn legacy_columns(connection: &Connection, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare(&format!("SELECT name FROM pragma_table_info('{table}', 'legacy')"))?;
    statement.query_map([], |row| row.get(0))?.collect()
}

