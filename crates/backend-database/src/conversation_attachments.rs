//! The attachments the agent uploaded (`commands.md` § Attachment commands,
//! `storage.md` § Conversation state and transactions): `demi attachment
//! upload` stores each file's bytes as a blob of the conversation owner's
//! namespace, and the conversation's `attachments` table holds one row per
//! attachment, keyed by its number in the conversation's `attachment`
//! sequence, with the file's name, its media type, its size and its blob. A
//! row is written once, or copied into a Fork's destination, and never
//! changes.

use std::fmt;
use std::str::FromStr;

use demi_shared_types::BlobRef;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::columns::decode;

const TABLE: &str = "attachments";

/// An attachment's number as the model and the page write it: `a3`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttachmentNumber(pub u64);

impl fmt::Display for AttachmentNumber {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "a{}", self.0)
    }
}

/// Text that names no attachment number.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0} is no attachment number such as a3")]
pub struct NotAnAttachment(String);

impl FromStr for AttachmentNumber {
    type Err = NotAnAttachment;

    /// `a` and a number from 1, written without leading zeros, as
    /// [`fmt::Display`] writes it.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refused = || NotAnAttachment(text.to_owned());
        let digits = text.strip_prefix('a').ok_or_else(refused)?;
        if digits.starts_with('0') || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(refused());
        }
        digits.parse().map(Self).map_err(|_| refused())
    }
}

/// One attachment's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentRow {
    pub number: AttachmentNumber,
    /// The file's name, without its directory.
    pub name: String,
    /// The media type the backend read from the bytes.
    pub media_type: String,
    pub size: u64,
    pub blob: BlobRef,
}

/// Writes `rows` in one transaction; a number that has a row keeps it.
pub fn insert(connection: &mut Connection, rows: &[AttachmentRow]) -> Result<(), StorageError> {
    let transaction = connection.transaction()?;
    {
        let mut insert = transaction.prepare_cached(
            "INSERT INTO attachments (number, name, media_type, size, blob)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (number) DO NOTHING",
        )?;
        for row in rows {
            insert.execute(params![
                column(row.number.0),
                row.name,
                row.media_type,
                column(row.size),
                row.blob.as_str(),
            ])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

/// The attachment `number`; none when the conversation has none by it.
pub fn read(
    connection: &Connection,
    number: AttachmentNumber,
) -> Result<Option<AttachmentRow>, StorageError> {
    connection
        .prepare_cached(
            "SELECT number, name, media_type, size, blob FROM attachments WHERE number = ?1",
        )?
        .query_row([column(number.0)], |row| Ok(decode_row(row)))
        .optional()?
        .transpose()
}

/// Every attachment of the conversation, by number, which a Fork copies into
/// its destination.
pub fn all(connection: &Connection) -> Result<Vec<AttachmentRow>, StorageError> {
    let mut statement = connection.prepare_cached(
        "SELECT number, name, media_type, size, blob FROM attachments ORDER BY number",
    )?;
    let mut rows = statement.query([])?;
    let mut attachments = Vec::new();
    while let Some(row) = rows.next()? {
        attachments.push(decode_row(row)?);
    }
    Ok(attachments)
}

/// A count as the INTEGER column holds it. Numbers come from a sequence and
/// sizes are at most an upload's, far below the column's range.
fn column(value: u64) -> i64 {
    i64::try_from(value).expect("an attachment's number and size fit the column")
}

fn decode_row(row: &Row<'_>) -> Result<AttachmentRow, StorageError> {
    let number = decode(TABLE, "number", u64::try_from(row.get::<_, i64>("number")?))?;
    Ok(AttachmentRow {
        number: AttachmentNumber(number),
        name: row.get("name")?,
        media_type: row.get("media_type")?,
        size: decode(TABLE, "size", u64::try_from(row.get::<_, i64>("size")?))?,
        blob: decode(
            TABLE,
            "blob",
            BlobRef::try_from(row.get::<_, String>("blob")?),
        )?,
    })
}
