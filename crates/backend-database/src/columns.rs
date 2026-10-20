//! How stored values become their types again.
//!
//! Every read decodes and validates what it reads, as it would any input from
//! outside the process. A value outside its type is corrupt, and the error names
//! its table and column. Nothing repairs or defaults it.

use std::fmt::Display;

use demi_shared_types::{Timestamp, WorkflowChoice};
use garde::Validate;
use rusqlite::Row;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::StorageError;

/// `value`, or the corruption of `table.column` that its error describes.
pub fn decode<T, E: Display>(
    table: &'static str,
    column: &'static str,
    value: Result<T, E>,
) -> Result<T, StorageError> {
    value.map_err(|error| StorageError::Corrupt {
        table,
        column,
        reason: error.to_string(),
    })
}

/// A stored millisecond count as a point in time.
pub fn instant(
    row: &Row<'_>,
    table: &'static str,
    column: &'static str,
) -> Result<Timestamp, StorageError> {
    decode(table, column, Timestamp::from_millisecond(row.get(column)?))
}

/// A JSON column's text as its type, checked by the type's rules.
pub fn json<T>(table: &'static str, column: &'static str, text: &str) -> Result<T, StorageError>
where
    T: DeserializeOwned + Validate<Context = ()>,
{
    decode(table, column, demi_shared_types::decode(text))
}

/// The text a JSON column holds for `value`.
pub fn to_json<T: Serialize>(value: &T) -> String {
    // The stored types serialize their fields as JSON strings, numbers,
    // arrays and objects with string keys, which serde_json never refuses.
    serde_json::to_string(value).expect("a stored value serializes to JSON")
}

/// A Workflow menu choice as its typed columns: the kind, and the
/// workflow's id for a workflow.
pub fn workflow_columns(choice: &WorkflowChoice) -> (&'static str, Option<&str>) {
    match choice {
        WorkflowChoice::Default => ("default", None),
        WorkflowChoice::None => ("none", None),
        WorkflowChoice::Workflow { id } => ("workflow", Some(id)),
    }
}

/// The choice a row's workflow columns `kind` and `id` hold, the kind read
/// from `table.column`.
pub fn workflow_choice(
    table: &'static str,
    column: &'static str,
    kind: &str,
    id: Option<String>,
) -> Result<WorkflowChoice, StorageError> {
    match (kind, id) {
        ("default", None) => Ok(WorkflowChoice::Default),
        ("none", None) => Ok(WorkflowChoice::None),
        ("workflow", Some(id)) => Ok(WorkflowChoice::Workflow { id }),
        _ => Err(StorageError::Corrupt {
            table,
            column,
            reason: format!("the kind {kind} does not fit its id"),
        }),
    }
}

/// A count or index as the INTEGER column holds it, such as a block's index
/// or a medium's place in its block; no transcript comes near `i64::MAX`
/// rows.
pub fn count(value: usize) -> i64 {
    i64::try_from(value).expect("a count fits the column")
}
