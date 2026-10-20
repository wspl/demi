//! A conversation's side questions as the web app lists them.
//!
//! The module stores each `side_questions` row, which the tree store writes with
//! its node, and the two fields the backend changes afterwards: the title the
//! small model writes once, and whether the card is folded.

use demi_shared_types::{BlockId, MediaSource, NodeId};
use demi_web_api_protocol::side_questions::SideQuestion;
use rusqlite::{Connection, Row, params};

use super::StorageError;
use super::columns::{decode, instant};

const TABLE: &str = "side_questions";

/// Every side question, in the order they were asked.
pub fn all(connection: &Connection) -> Result<Vec<SideQuestion>, StorageError> {
    let mut statement = connection.prepare(
        "SELECT node_id, block_id, start_offset, end_offset, quote, picture, question, asked_at, title, folded
         FROM side_questions ORDER BY asked_at, node_id",
    )?;
    let mut rows = statement.query([])?;
    let mut questions = Vec::new();
    while let Some(row) = rows.next()? {
        questions.push(question(row)?);
    }
    Ok(questions)
}

/// The side question `id`; none when the conversation has no such one.
pub fn one(connection: &Connection, id: &NodeId) -> Result<Option<SideQuestion>, StorageError> {
    let mut statement = connection.prepare(
        "SELECT node_id, block_id, start_offset, end_offset, quote, picture, question, asked_at, title, folded
         FROM side_questions WHERE node_id = ?1",
    )?;
    let mut rows = statement.query([id.as_str()])?;
    rows.next()?.map(question).transpose()
}

/// Writes the small model's title of the side question `id`, unless it has
/// one already; answers whether it wrote it.
pub fn set_title(connection: &Connection, id: &NodeId, title: &str) -> Result<bool, StorageError> {
    let changed = connection.execute(
        "UPDATE side_questions SET title = ?2 WHERE node_id = ?1 AND title IS NULL",
        params![id.as_str(), title],
    )?;
    Ok(changed > 0)
}

/// Folds the card of the side question `id`, or opens it; answers the side
/// question as it is now, none when the conversation has no such one.
pub fn set_folded(connection: &Connection, id: &NodeId, folded: bool) -> Result<Option<SideQuestion>, StorageError> {
    connection.execute(
        "UPDATE side_questions SET folded = ?2 WHERE node_id = ?1",
        params![id.as_str(), folded],
    )?;
    one(connection, id)
}

/// The picture a side question about a picture asks of, from its JSON
/// (`picture`: a media source); none for one about words.
pub(crate) fn picture(row: &Row<'_>, table: &'static str) -> Result<Option<MediaSource>, StorageError> {
    row.get::<_, Option<String>>("picture")?
        .map(|json| decode(table, "picture", serde_json::from_str::<MediaSource>(&json).map_err(|error| error.to_string())))
        .transpose()
}

fn question(row: &Row<'_>) -> Result<SideQuestion, StorageError> {
    Ok(SideQuestion {
        id: decode(TABLE, "node_id", NodeId::try_from(row.get::<_, String>("node_id")?))?,
        block_id: decode(TABLE, "block_id", BlockId::try_from(row.get::<_, String>("block_id")?))?,
        start: decode(TABLE, "start_offset", u64::try_from(row.get::<_, i64>("start_offset")?))?,
        end: decode(TABLE, "end_offset", u64::try_from(row.get::<_, i64>("end_offset")?))?,
        quote: row.get("quote")?,
        picture: picture(row, TABLE)?,
        question: row.get("question")?,
        title: row.get("title")?,
        folded: row.get("folded")?,
        asked_at: instant(row, TABLE, "asked_at")?,
    })
}
