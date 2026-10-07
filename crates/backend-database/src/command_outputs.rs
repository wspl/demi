//! The records of commands' outputs (`storage.md` § Command outputs): each
//! ended command's whole output is a blob of the conversation owner's
//! namespace, in the kept output's records, and the conversation's
//! `command_outputs` table holds one row per command, keyed by its id,
//! which says when the command ended and whether its output is stored, with
//! its media, or was not stored and why. A row is written once when its
//! command ends, or copied into a Fork's destination, and never changes.

use demi_host_interface::{Missing, StoredMedium};
use demi_shared_types::{BlobRef, Block, CommandId, Timestamp, ToolView};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{decode, instant, json, to_json};

/// What a conversation holds of an ended command's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputRow {
    /// Its blob, the bytes at its end that the backend does not have, and
    /// the command's media by number.
    Stored {
        blob: BlobRef,
        missing: Option<Missing>,
        media: Vec<StoredMedium>,
    },
    /// Why it was not stored.
    NotStored(String),
}

/// A command's row: when it ended, and its output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub command: CommandId,
    pub ended: Timestamp,
    pub output: OutputRow,
}

const COLUMNS: &str = "command_id, ended_at, blob, missing_bytes, missing_reason, media, not_stored";

/// Writes `rows` in one transaction; a command that has a row keeps it.
pub fn insert(connection: &mut Connection, rows: &[CommandOutput]) -> Result<(), StorageError> {
    let transaction = connection.transaction()?;
    {
        let mut insert = transaction.prepare_cached(&format!(
            "INSERT INTO command_outputs ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (command_id) DO NOTHING"
        ))?;
        for row in rows {
            let (blob, missing, media, not_stored) = match &row.output {
                OutputRow::Stored {
                    blob,
                    missing,
                    media,
                } => (Some(blob), missing.as_ref(), Some(media), None),
                OutputRow::NotStored(reason) => (None, None, None, Some(reason)),
            };
            let missing_bytes =
                missing.map(|missing| i64::try_from(missing.bytes).unwrap_or(i64::MAX));
            insert.execute(params![
                row.command.as_str(),
                row.ended.as_millisecond(),
                blob.map(BlobRef::as_str),
                missing_bytes,
                missing.map(|missing| missing.reason.as_str()),
                media.map(to_json),
                not_stored,
            ])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

/// The row of `command`; none when the conversation has none.
pub fn read(
    connection: &Connection,
    command: &CommandId,
) -> Result<Option<CommandOutput>, StorageError> {
    connection
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM command_outputs WHERE command_id = ?1"
        ))?
        .query_row([command.as_str()], output_row)
        .optional()?
        .transpose()
}

/// Every row of the conversation.
pub fn all(connection: &Connection) -> Result<Vec<CommandOutput>, StorageError> {
    let mut statement =
        connection.prepare_cached(&format!("SELECT {COLUMNS} FROM command_outputs"))?;
    let mut rows = statement.query([])?;
    let mut outputs = Vec::new();
    while let Some(row) = rows.next()? {
        outputs.push(decode_row(row)?);
    }
    Ok(outputs)
}

/// The rows of `commands`, which a Fork copies into its destination.
pub fn rows(
    connection: &Connection,
    commands: &[CommandId],
) -> Result<Vec<CommandOutput>, StorageError> {
    let mut rows = Vec::new();
    for command in commands {
        if let Some(row) = read(connection, command)? {
            rows.push(row);
        }
    }
    Ok(rows)
}

/// The commands the shell calls of `blocks` name, each once.
pub fn commands_of(blocks: &[Block]) -> Vec<CommandId> {
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

/// A stored output's media, read back from their column.
fn stored_media(text: String) -> Result<Vec<StoredMedium>, StorageError> {
    json("command_outputs", "media", &text)
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
    let media: Option<String> = row.get("media")?;
    let not_stored: Option<String> = row.get("not_stored")?;
    let output = match (stored, not_stored) {
        (Some(stored), None) => {
            let media = stored_media(media.ok_or_else(|| StorageError::Corrupt {
                table: "command_outputs",
                column: "media",
                reason: "a stored output comes with its media".into(),
            })?)?;
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
                media,
            }
        }
        (None, Some(reason)) => OutputRow::NotStored(reason),
        _ => {
            return Err(StorageError::Corrupt {
                table: "command_outputs",
                column: "blob",
                reason: "a row is stored or not stored".into(),
            });
        }
    };
    Ok(CommandOutput {
        command,
        ended,
        output,
    })
}

fn blob(text: String) -> Result<BlobRef, StorageError> {
    decode("command_outputs", "blob", BlobRef::try_from(text))
}
