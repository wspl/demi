//! From 0.1.5785's conversation schema: blocks are written in transcript
//! block format 19 and checkpoints in checkpoint state format 11, whose
//! comments hold content, a message's, where they held their words as
//! `text`. A record names its format, and format 18 is still read as 19; the
//! step rewrites each queued message's comments in the nodes' states, and
//! each side question's question, its words until now, as a message's
//! content.

use rusqlite::Transaction;
use serde_json::Value;

use super::super::{Migration, ReadBlob, corrupt_text};

pub(super) const MIGRATION: Migration = Migration::Code(conversation_from_0_1_5785);

fn conversation_from_0_1_5785(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    let states: Vec<(String, String)> = transaction
        .prepare("SELECT id, state FROM nodes")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut update = transaction.prepare("UPDATE nodes SET state = ?1 WHERE id = ?2")?;
    for (id, state) in states {
        let mut value: Value = serde_json::from_str(&state)
            .map_err(|error| corrupt_text(&format!("a node's state is not JSON: {error}")))?;
        let mut changed = false;
        if let Some(Value::Array(queue)) = value.get_mut("queue") {
            for message in queue {
                if let Some(Value::Array(comments)) = message.get_mut("comments") {
                    for comment in comments {
                        changed |= contented(comment);
                    }
                }
            }
        }
        if changed {
            update.execute(rusqlite::params![value.to_string(), id])?;
        }
    }
    transaction.execute_batch(
        "UPDATE side_questions SET question = json_array(json_object('type', 'text', 'text', question));",
    )?;
    Ok(())
}

/// A comment with its words as text content; false for one that holds no
/// words of its own.
fn contented(comment: &mut Value) -> bool {
    let Value::Object(fields) = comment else {
        return false;
    };
    let Some(text) = fields.remove("text") else {
        return false;
    };
    fields.insert("content".to_owned(), serde_json::json!([{ "type": "text", "text": text }]));
    true
}
