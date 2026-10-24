//! The conversion of a conversation's database of the last schema whose rows
//! held whole blocks ([`schema::LEGACY_CONVERSATION`]) into the current one,
//! whose rows are records that name their contents (`records.rs`).
//!
//! ```text
//! <id>.sqlite             the old file, read and never written
//!      |  stream its rows, one at a time, a few hundred to a transaction
//!      v
//! <id>.sqlite.converting  the new file: each row read back and compared
//!      |  rename, once every row compared equal
//!      v
//! <id>.sqlite             the new file
//! ```
//!
//! The old file stays as it was until the rename replaces it, so an upgrade's
//! snapshot holds it by a hard link, and a conversion that stops at any step
//! starts over from it. Every row, every node and every path stay as they
//! were: only how a row is stored changes.

use std::path::{Path as FilePath, PathBuf};

use demi_shared_types::Block;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use super::columns::json;
use super::records::{self, Codec, Stored};
use super::schema::{self, LegacyForm, ReadBlob};
use super::{StorageError, sqlite};

/// How many rows one transaction of the new file writes: few enough that
/// no transaction holds much, many enough that commits cost little.
const ROWS_PER_TRANSACTION: usize = 500;

/// How many rows the dictionary is trained on, spread over the file.
const SAMPLES: i64 = 3000;

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
/// the file's opening turn, and no connection has it open.
pub(crate) fn convert_if_due(path: &FilePath, blobs: &dyn ReadBlob) -> Result<(), StorageError> {
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
    let converted = convert(path, form);
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

/// Writes the new file beside `path`, compares it and renames it over
/// `path`; answers how many rows it converted.
fn convert(path: &FilePath, form: LegacyForm) -> Result<u64, StorageError> {
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
    {
        let transaction = new.transaction()?;
        for table in COPIED {
            transaction.execute(&format!("INSERT INTO main.{table} SELECT * FROM legacy.{table}"), [])?;
        }
        if form == LegacyForm::AutomationNames {
            schema::name_schedules(&transaction, "nodes", "state")?;
        }
        if let Some(dictionary) = dictionary(&legacy, form)? {
            transaction.execute("INSERT INTO dictionaries (bytes) VALUES (?1)", [dictionary])?;
        }
        transaction.commit()?;
    }
    let rows = convert_rows(&legacy, &mut new, form)?;
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

/// A dictionary trained on records of rows spread over the old file.
fn dictionary(legacy: &Connection, form: LegacyForm) -> Result<Option<Vec<u8>>, StorageError> {
    let last: Option<i64> = legacy.query_row("SELECT max(rowid) FROM blocks", [], |row| row.get(0))?;
    let Some(last) = last.filter(|last| *last >= records::DICTIONARY_AFTER) else {
        return Ok(None);
    };
    let mut statement = legacy.prepare("SELECT block FROM blocks WHERE rowid >= ?1 ORDER BY rowid LIMIT 1")?;
    let mut samples = Vec::new();
    for at in 0..SAMPLES {
        let text: Option<String> = statement.query_row([last * at / SAMPLES], |row| row.get(0)).optional()?;
        if let Some(text) = text {
            let block = legacy_block("blocks", &text, form)?;
            samples.push(records::split(&block)?.body);
        }
    }
    Ok(records::train(&samples))
}

/// Writes a record for each row of the old file, compares it with the row,
/// and answers how many it wrote.
fn convert_rows(legacy: &Connection, new: &mut Connection, form: LegacyForm) -> Result<u64, StorageError> {
    let mut rows = legacy.prepare("SELECT node_id, idx, block, follows, replaces FROM blocks NOT INDEXED ORDER BY rowid")?;
    let mut rows = rows.query([])?;
    let mut written = 0u64;
    loop {
        let transaction = new.transaction()?;
        let codec = Codec::new(&transaction);
        let mut batch = 0;
        while batch < ROWS_PER_TRANSACTION {
            let Some(row) = rows.next()? else {
                break;
            };
            let (node, idx, text): (String, i64, String) = (row.get(0)?, row.get(1)?, row.get(2)?);
            let (follows, replaces): (Option<i64>, Option<i64>) = (row.get(3)?, row.get(4)?);
            batch += 1;
            let block = legacy_block("blocks", &text, form)?;
            records::insert(&transaction, &codec, &node, idx, follows, replaces, &block)?;
            let stored = transaction
                .prepare_cached("SELECT format, dictionary, body FROM blocks WHERE node_id = ?1 AND idx = ?2")?
                .query_row(params![node, idx], |row| Stored::of(row, 0))?;
            if codec.block("blocks", &stored)? != block {
                return Err(StorageError::Corrupt {
                    table: "blocks",
                    column: "body",
                    reason: format!("the row {idx} of node {node} does not read back as it was"),
                });
            }
            written += 1;
        }
        drop(codec);
        transaction.commit()?;
        if batch < ROWS_PER_TRANSACTION {
            return Ok(written);
        }
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
