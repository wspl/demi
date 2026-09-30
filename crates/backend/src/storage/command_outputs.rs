//! The records of commands' outputs (`storage.md` § Command outputs): each
//! ended command's whole output is a blob of the conversation owner's
//! namespace, in the kept output's records, and the conversation's
//! `command_outputs` table holds one row per command, keyed by its id,
//! which says when the command ended and whether its output is stored, was
//! not stored and why, or was removed and when. A row is written once when
//! its command ends, or copied into a Fork's destination, and changes only
//! when the retention pass removes its output. The blobs whose rows a commit
//! writes or removes are its uses, which it records before it commits
//! (`storage.md` § Collecting blobs).

use demi_agent_store::StoreError;
use demi_core::{BlobRef, Block, CommandId, Timestamp, ToolView};
use demi_shell::Missing;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::blobs::UserBlobs;
use super::columns::{decode, instant};

/// What a conversation holds of an ended command's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OutputRow {
    /// Its blob, and the bytes at its end that the backend does not have.
    Stored { blob: BlobRef, missing: Option<Missing> },
    /// Why it was not stored.
    NotStored(String),
    /// When the retention pass removed it.
    Removed(Timestamp),
}

/// A command's row: when it ended, and its output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandOutput {
    pub(crate) command: CommandId,
    pub(crate) ended: Timestamp,
    pub(crate) output: OutputRow,
}

const COLUMNS: &str = "command_id, ended_at, blob, missing_bytes, missing_reason, not_stored, removed_at";

/// Writes `rows` in one transaction; a command that has a row keeps it. The
/// blobs they name are used before the commit.
pub(crate) fn insert(
    connection: &mut Connection,
    blobs: &UserBlobs,
    rows: &[CommandOutput],
) -> Result<Result<(), StoreError>, StorageError> {
    let transaction = connection.transaction()?;
    let mut touched = Vec::new();
    {
        let mut insert = transaction.prepare_cached(&format!(
            "INSERT INTO command_outputs ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (command_id) DO NOTHING"
        ))?;
        for row in rows {
            let (blob, missing, not_stored, removed) = match &row.output {
                OutputRow::Stored { blob, missing } => (Some(blob), missing.as_ref(), None, None),
                OutputRow::NotStored(reason) => (None, None, Some(reason), None),
                OutputRow::Removed(at) => (None, None, None, Some(at.as_millisecond())),
            };
            let missing_bytes = missing.map(|missing| i64::try_from(missing.bytes).unwrap_or(i64::MAX));
            insert.execute(params![
                row.command.as_str(),
                row.ended.as_millisecond(),
                blob.map(BlobRef::as_str),
                missing_bytes,
                missing.map(|missing| missing.reason.as_str()),
                not_stored,
                removed,
            ])?;
            touched.extend(blob.cloned());
        }
    }
    if let Err(refused) = blobs.commit_uses(&touched) {
        return Ok(Err(refused));
    }
    transaction.commit()?;
    Ok(Ok(()))
}

/// The row of `command`; none when the conversation has none.
pub(crate) fn read(connection: &Connection, command: &CommandId) -> Result<Option<CommandOutput>, StorageError> {
    connection
        .prepare_cached(&format!("SELECT {COLUMNS} FROM command_outputs WHERE command_id = ?1"))?
        .query_row([command.as_str()], output_row)
        .optional()?
        .transpose()
}

/// The rows of `commands`, which a Fork copies into its destination.
pub(crate) fn rows(connection: &Connection, commands: &[CommandId]) -> Result<Vec<CommandOutput>, StorageError> {
    let mut rows = Vec::new();
    for command in commands {
        if let Some(row) = read(connection, command)? {
            rows.push(row);
        }
    }
    Ok(rows)
}

/// The commands the shell calls of `blocks` name, each once.
pub(crate) fn commands_of(blocks: &[Block]) -> Vec<CommandId> {
    let mut commands: Vec<CommandId> = Vec::new();
    for block in blocks {
        if let Block::ToolCall(call) = block
            && let Some(ToolView::Shell(view)) = &call.view
            && !commands.contains(&view.command_id)
        {
            commands.push(view.command_id.clone());
        }
    }
    commands
}

/// Whether an output stored for a command that ended before `expired` is
/// there to remove.
pub(crate) fn expired(connection: &Connection, expired: Timestamp) -> Result<bool, StorageError> {
    Ok(connection
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM command_outputs WHERE blob IS NOT NULL AND ended_at < ?1)")?
        .query_row([expired.as_millisecond()], |row| row.get(0))?)
}

/// Marks removed at `now` the stored outputs of the commands that ended
/// before `expired`, in one transaction (`storage.md` § Removing command
/// outputs). The blobs they let go of are used before the commit, so a read
/// of a row just before still finds its blob. Answers how many it removed.
pub(crate) fn remove_expired(
    connection: &mut Connection,
    blobs: &UserBlobs,
    expired: Timestamp,
    now: Timestamp,
) -> Result<Result<usize, StoreError>, StorageError> {
    let transaction = connection.transaction()?;
    let expired = expired.as_millisecond();
    // SQLite's RETURNING answers the new values, so the blobs are read
    // first, in the same transaction.
    let mut released = Vec::new();
    {
        let mut select = transaction
            .prepare_cached("SELECT blob FROM command_outputs WHERE blob IS NOT NULL AND ended_at < ?1")?;
        let mut rows = select.query([expired])?;
        while let Some(row) = rows.next()? {
            released.push(blob(row.get(0)?)?);
        }
    }
    let removed = transaction
        .prepare_cached(
            "UPDATE command_outputs
             SET blob = NULL, missing_bytes = NULL, missing_reason = NULL, removed_at = ?2
             WHERE blob IS NOT NULL AND ended_at < ?1",
        )?
        .execute(params![expired, now.as_millisecond()])?;
    if let Err(refused) = blobs.commit_uses(&released) {
        return Ok(Err(refused));
    }
    transaction.commit()?;
    Ok(Ok(removed))
}

/// The blobs the rows hold.
pub(crate) fn references(connection: &Connection) -> Result<Vec<BlobRef>, StorageError> {
    let mut statement = connection.prepare_cached("SELECT blob FROM command_outputs WHERE blob IS NOT NULL")?;
    let mut rows = statement.query([])?;
    let mut references = Vec::new();
    while let Some(row) = rows.next()? {
        references.push(blob(row.get(0)?)?);
    }
    Ok(references)
}

/// A row read back, decoded and checked.
fn output_row(row: &Row<'_>) -> rusqlite::Result<Result<CommandOutput, StorageError>> {
    Ok(decode_row(row))
}

fn decode_row(row: &Row<'_>) -> Result<CommandOutput, StorageError> {
    let command = decode(
        "command_outputs",
        "command_id",
        CommandId::try_from(row.get::<_, String>("command_id")?),
    )?;
    let ended = instant(row, "command_outputs", "ended_at")?;
    let stored: Option<String> = row.get("blob")?;
    let missing_bytes: Option<i64> = row.get("missing_bytes")?;
    let missing_reason: Option<String> = row.get("missing_reason")?;
    let not_stored: Option<String> = row.get("not_stored")?;
    let removed: Option<i64> = row.get("removed_at")?;
    let output = match (stored, not_stored, removed) {
        (Some(stored), None, None) => {
            let missing = match (missing_bytes, missing_reason) {
                (Some(bytes), Some(reason)) => Some(Missing {
                    bytes: decode("command_outputs", "missing_bytes", u64::try_from(bytes))?,
                    reason,
                }),
                (None, None) => None,
                _ => {
                    return Err(StorageError::Corrupt {
                        table: "command_outputs",
                        column: "missing_reason",
                        reason: "missing bytes come with their reason".into(),
                    });
                }
            };
            OutputRow::Stored {
                blob: blob(stored)?,
                missing,
            }
        }
        (None, Some(reason), None) => OutputRow::NotStored(reason),
        (None, None, Some(at)) => OutputRow::Removed(decode(
            "command_outputs",
            "removed_at",
            Timestamp::from_millisecond(at),
        )?),
        _ => {
            return Err(StorageError::Corrupt {
                table: "command_outputs",
                column: "blob",
                reason: "a row is stored, not stored or removed".into(),
            });
        }
    };
    Ok(CommandOutput { command, ended, output })
}

fn blob(text: String) -> Result<BlobRef, StorageError> {
    decode("command_outputs", "blob", BlobRef::try_from(text))
}
