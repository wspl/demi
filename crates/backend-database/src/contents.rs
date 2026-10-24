//! A conversation's contents: the bytes its records name, each kept once.
//!
//! A content is a string, the bytes of a base64 string, or a JSON value of
//! a block (`records.rs`), kept in the `contents` table of the
//! conversation's database under the SHA-256 of its bytes, compressed with
//! zstd. A content never changes. `block_contents` and `share_contents`
//! list what each record names, so a content is removed in the transaction
//! that drops the last record naming it, and the foreign keys refuse to
//! remove one that a record still names.

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest as _, Sha256};

use super::StorageError;

/// The SHA-256 of a content's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Hash([u8; 32]);

impl Hash {
    pub(crate) fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.0
    }

    pub(crate) fn hex(&self) -> String {
        hex::encode(self.0)
    }

    pub(crate) fn from_hex(text: &str) -> Option<Self> {
        let mut hash = [0u8; 32];
        hex::decode_to_slice(text, &mut hash).ok()?;
        Some(Self(hash))
    }
}

/// zstd's level for contents and records: its default, which takes a
/// fraction of a millisecond for the few kilobytes most are.
pub(crate) const LEVEL: i32 = 3;

/// The id of the content `bytes`, written compressed when the database does
/// not hold it yet.
pub(crate) fn put(transaction: &Connection, hash: &Hash, bytes: &[u8]) -> Result<i64, StorageError> {
    if let Some(id) = id_of(transaction, hash)? {
        return Ok(id);
    }
    let stored = zstd::bulk::compress(bytes, LEVEL)?;
    Ok(transaction
        .prepare_cached("INSERT INTO contents (hash, bytes) VALUES (?1, ?2) RETURNING id")?
        .query_row(params![hash.as_slice(), stored], |row| row.get(0))?)
}

/// The id of the content `hash`; none when the database holds none.
pub(crate) fn id_of(connection: &Connection, hash: &Hash) -> Result<Option<i64>, StorageError> {
    Ok(connection
        .prepare_cached("SELECT id FROM contents WHERE hash = ?1")?
        .query_row([hash.as_slice()], |row| row.get(0))
        .optional()?)
}

/// The bytes of the content `hash`, or corrupt data when the database holds
/// none, since a record names only contents written with it.
pub(crate) fn read(connection: &Connection, hash: &Hash) -> Result<Vec<u8>, StorageError> {
    let stored: Option<Vec<u8>> = connection
        .prepare_cached("SELECT bytes FROM contents WHERE hash = ?1")?
        .query_row([hash.as_slice()], |row| row.get(0))
        .optional()?;
    let stored = stored.ok_or_else(|| corrupt(format!("the content {} is missing", hash.hex())))?;
    decompress(&stored, None)
}

/// Removes those of `contents` that no record names any longer, and gives
/// the pages the transaction freed back to the file system.
pub(crate) fn remove_unnamed(transaction: &Connection, contents: impl IntoIterator<Item = i64>) -> Result<(), StorageError> {
    let mut remove = transaction.prepare_cached(
        "DELETE FROM contents WHERE id = ?1
           AND NOT EXISTS (SELECT 1 FROM block_contents WHERE content = ?1)
           AND NOT EXISTS (SELECT 1 FROM share_contents WHERE content = ?1)",
    )?;
    let mut removed = 0;
    for content in contents {
        removed += remove.execute([content])?;
    }
    if removed > 0 {
        // The pragma frees one page each time it is stepped.
        let mut vacuum = transaction.prepare_cached("PRAGMA incremental_vacuum")?;
        let mut steps = vacuum.query([])?;
        while steps.next()?.is_some() {}
    }
    Ok(())
}

/// `stored` decompressed, with the dictionary it was compressed with.
pub(crate) fn decompress(
    stored: &[u8],
    decompressor: Option<&mut zstd::bulk::Decompressor<'static>>,
) -> Result<Vec<u8>, StorageError> {
    let size = zstd::zstd_safe::get_frame_content_size(stored)
        .ok()
        .flatten()
        .and_then(|size| usize::try_from(size).ok())
        .ok_or_else(|| corrupt("a stored value does not say its size".into()))?;
    Ok(match decompressor {
        Some(decompressor) => decompressor.decompress(stored, size)?,
        None => zstd::bulk::decompress(stored, size)?,
    })
}

fn corrupt(reason: String) -> StorageError {
    StorageError::Corrupt {
        table: "contents",
        column: "bytes",
        reason,
    }
}
