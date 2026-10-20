//! The conversation's attachments, the files its messages sent and those the
//! agent uploaded
//! (`crates/backend-host-access/src/attachment_commands.rs`).
//!
//! A sent file's bytes, and those `demi attachment upload` stores, are a blob of
//! the conversation owner's namespace. The conversation's `attachments` table
//! holds one row per attachment, keyed by its number in the conversation's
//! `attachment` sequence, with:
//!
//! - the file's name, its media type and its size;
//! - an image's or a video's size in pixels;
//! - its blob;
//! - the node of the agent that uploaded it, none for a file the user sent;
//! - the file an upload was copied from, its device and its full path, which
//!   the page opens in the copy's place while it can read it
//!   (`decisions/0003-attachment-opens-its-file.md`); none for a file the user
//!   sent, and never in a share's snapshot, which shows only the copy.
//!
//! A row is written once, or copied into a Fork's destination, and never
//! changes.
//!
//! A reply shows only the attachments its own agent uploaded, or the agent it was
//! forked from for the history it copied. The page asks for each as
//! [`read_shown`] reads it, and a share keeps each as [`SHOWN`] pairs it.

use std::fmt;
use std::str::FromStr;

use demi_shared_types::{BlobRef, NodeId, PixelSize};
use demi_web_api_protocol::ids::DeviceId;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

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
    /// [`fmt::Display`] writes it, within the range the column holds, so a
    /// number past it names no attachment rather than failing a query.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refused = || NotAnAttachment(text.to_owned());
        let digits = text.strip_prefix('a').ok_or_else(refused)?;
        if digits.starts_with('0') || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(refused());
        }
        let number: i64 = digits.parse().map_err(|_| refused())?;
        u64::try_from(number).map(Self).map_err(|_| refused())
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
    /// An image's or a video's size in pixels, read from its header when
    /// the row was written; none for any other file, and for one whose
    /// header does not give it.
    pub pixels: Option<PixelSize>,
    pub blob: BlobRef,
    /// The file it was copied from; none for a file the user sent, for an
    /// upload stored before uploads kept it, and in a share's snapshot.
    pub source: Option<AttachmentSource>,
}

/// The file of a Host an attachment was copied from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentSource {
    /// The device whose runner ran the upload.
    pub host: DeviceId,
    /// The file's full path, as that Host writes it.
    pub path: String,
}

/// An attachment's row with who stored it: the node of the agent whose
/// `demi attachment upload` did, a side question's included; none for a
/// file the user sent, or uploaded from a command the user ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAttachment {
    pub row: AttachmentRow,
    pub node: Option<NodeId>,
}

/// The table `shown (node, uploader)`, a recursive common table expression a
/// statement starts `WITH`.
///
/// The replies of `node` show what `uploader` uploaded, for each node and
/// itself, and for each node and every agent it was forked from, recursively,
/// since the history a forked agent copied was written by the agent it was forked
/// from. A file the user sent, or another agent uploaded, is no file of its
/// replies.
pub const SHOWN: &str = "RECURSIVE shown (node, uploader) AS (
       SELECT id, id FROM nodes
       UNION
       SELECT shown.node, source.id FROM shown
         JOIN nodes AS forked ON forked.id = shown.uploader
         JOIN nodes AS source ON source.number = forked.forked_from
     )";

/// Writes `rows` in `transaction`, the caller's, which gives their numbers
/// or copies them; a number that has a row keeps it.
pub fn insert(transaction: &Transaction<'_>, rows: &[StoredAttachment]) -> Result<(), StorageError> {
    let mut insert = transaction.prepare_cached(
        "INSERT INTO attachments (number, name, media_type, size, width, height, blob, node_id, source_host, source_path)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT (number) DO NOTHING",
    )?;
    for StoredAttachment { row, node } in rows {
        insert.execute(params![
            column(row.number.0),
            row.name,
            row.media_type,
            column(row.size),
            row.pixels.map(|pixels| pixels.width),
            row.pixels.map(|pixels| pixels.height),
            row.blob.as_str(),
            node.as_ref().map(NodeId::as_str),
            row.source.as_ref().map(|source| source.host.as_str()),
            row.source.as_ref().map(|source| source.path.as_str()),
        ])?;
    }
    Ok(())
}

/// The attachment `number`; none when the conversation has none by it.
pub fn read(
    connection: &Connection,
    number: AttachmentNumber,
) -> Result<Option<AttachmentRow>, StorageError> {
    connection
        .prepare_cached(
            "SELECT number, name, media_type, size, width, height, blob, source_host, source_path
             FROM attachments WHERE number = ?1",
        )?
        .query_row([column(number.0)], |row| Ok(decode_row(row)))
        .optional()?
        .transpose()
}

/// The attachment `number` as a reply of the node `node` shows it: none when
/// the conversation has none by it, or [`SHOWN`] does not pair the node
/// with its uploader.
pub fn read_shown(
    connection: &Connection,
    number: AttachmentNumber,
    node: &NodeId,
) -> Result<Option<AttachmentRow>, StorageError> {
    connection
        .prepare_cached(&format!(
            "WITH {SHOWN}
             SELECT number, name, media_type, size, width, height, blob, source_host, source_path
             FROM attachments AS attachment JOIN shown ON shown.uploader = attachment.node_id
             WHERE attachment.number = ?1 AND shown.node = ?2"
        ))?
        .query_row(params![column(number.0), node.as_str()], |row| Ok(decode_row(row)))
        .optional()?
        .transpose()
}

/// The attachment `number` as the snapshot of the share `token` keeps it;
/// none when it keeps none by it.
pub fn read_shared(
    connection: &Connection,
    token: &str,
    number: AttachmentNumber,
) -> Result<Option<AttachmentRow>, StorageError> {
    connection
        .prepare_cached(
            "SELECT number, name, media_type, size, width, height, blob,
               NULL AS source_host, NULL AS source_path
             FROM share_attachments WHERE token = ?1 AND number = ?2",
        )?
        .query_row(params![token, column(number.0)], |row| Ok(decode_row(row)))
        .optional()?
        .transpose()
}

/// The number of the conversation's latest attachment, 0 before its first.
/// Numbers are given and their rows written in one transaction, so every
/// number up to it that has no row never will.
pub fn last(connection: &Connection) -> Result<u64, StorageError> {
    let number: i64 = connection
        .prepare_cached("SELECT coalesce(max(number), 0) FROM attachments")?
        .query_row([], |row| row.get(0))?;
    decode(TABLE, "number", u64::try_from(number))
}

/// Every attachment of the conversation, by number, with who stored it,
/// which a Fork copies into its destination.
pub fn all(connection: &Connection) -> Result<Vec<StoredAttachment>, StorageError> {
    let mut statement = connection.prepare_cached(
        "SELECT number, name, media_type, size, width, height, blob, node_id, source_host, source_path
         FROM attachments ORDER BY number",
    )?;
    let mut rows = statement.query([])?;
    let mut attachments = Vec::new();
    while let Some(row) = rows.next()? {
        let node: Option<String> = row.get("node_id")?;
        attachments.push(StoredAttachment {
            row: decode_row(row)?,
            node: node.map(|node| decode(TABLE, "node_id", NodeId::try_from(node))).transpose()?,
        });
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
        pixels: decode_pixels(row)?,
        blob: decode(
            TABLE,
            "blob",
            BlobRef::try_from(row.get::<_, String>("blob")?),
        )?,
        source: decode_source(row)?,
    })
}

/// A row's source, which it holds both halves of or neither.
fn decode_source(row: &Row<'_>) -> Result<Option<AttachmentSource>, StorageError> {
    let host: Option<String> = row.get("source_host")?;
    let path: Option<String> = row.get("source_path")?;
    match (host, path) {
        (Some(host), Some(path)) => Ok(Some(AttachmentSource {
            host: decode(TABLE, "source_host", DeviceId::try_from(host))?,
            path,
        })),
        (None, None) => Ok(None),
        _ => decode(TABLE, "source_path", Err("a row holds a source's host and path, or neither")),
    }
}

/// A row's width and height, which it holds both or neither of.
fn decode_pixels(row: &Row<'_>) -> Result<Option<PixelSize>, StorageError> {
    let width: Option<u32> = row.get("width")?;
    let height: Option<u32> = row.get("height")?;
    match (width, height) {
        (Some(width), Some(height)) => Ok(Some(PixelSize { width, height })),
        (None, None) => Ok(None),
        _ => decode(TABLE, "height", Err("a row holds a width and a height, or neither")),
    }
}
