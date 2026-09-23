//! How stored values become their types again (`storage.md` § Encodings and
//! digests). Every read decodes and validates what it reads, as it would any
//! input from outside the process; a value outside its type is corrupt, and
//! the error names its table and column. Nothing repairs or defaults it.

use std::fmt::Display;

use demi_core::Timestamp;
use garde::Validate;
use rusqlite::Row;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::StorageError;

/// `value`, or the corruption of `table.column` that its error describes.
pub(crate) fn decode<T, E: Display>(table: &'static str, column: &'static str, value: Result<T, E>) -> Result<T, StorageError> {
    value.map_err(|error| StorageError::Corrupt {
        table,
        column,
        reason: error.to_string(),
    })
}

/// A stored millisecond count as a point in time.
pub(crate) fn instant(row: &Row<'_>, table: &'static str, column: &'static str) -> Result<Timestamp, StorageError> {
    decode(table, column, Timestamp::from_millisecond(row.get(column)?))
}

/// A JSON column's text as its type, checked by the type's rules.
pub(crate) fn json<T>(table: &'static str, column: &'static str, text: &str) -> Result<T, StorageError>
where
    T: DeserializeOwned + Validate<Context = ()>,
{
    decode(table, column, demi_core::decode(text))
}

/// The text a JSON column holds for `value`.
pub(crate) fn to_json<T: Serialize>(value: &T) -> String {
    // The stored types serialize their fields as JSON strings, numbers,
    // arrays and objects with string keys, which serde_json never refuses.
    serde_json::to_string(value).expect("a stored value serializes to JSON")
}
