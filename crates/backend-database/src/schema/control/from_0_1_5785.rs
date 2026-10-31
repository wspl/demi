//! From 0.1.5785's control schema: drafts are kept in draft format 2, whose
//! waiting comments each hold a message as the draft's own, its text and its
//! files, where they held their words as `text`. The step rewrites each
//! draft's comments.

use rusqlite::Transaction;
use serde_json::Value;

use super::super::{Migration, ReadBlob, corrupt_text};

pub(super) const MIGRATION: Migration = Migration::Code(control_from_0_1_5785);

fn control_from_0_1_5785(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    let drafts: Vec<(String, String)> = transaction
        .prepare("SELECT conversation_id, document FROM conversation_drafts")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut update = transaction.prepare("UPDATE conversation_drafts SET document = ?1 WHERE conversation_id = ?2")?;
    for (id, document) in drafts {
        let mut value: Value = serde_json::from_str(&document)
            .map_err(|error| corrupt_text(&format!("a draft is not JSON: {error}")))?;
        let mut changed = false;
        if let Some(Value::Array(comments)) = value.get_mut("comments") {
            for comment in comments {
                if let Value::Object(fields) = comment
                    && let Some(text) = fields.remove("text")
                {
                    fields.insert("content".to_owned(), serde_json::json!({ "text": text, "files": [] }));
                    changed = true;
                }
            }
        }
        if changed {
            update.execute(rusqlite::params![value.to_string(), id])?;
        }
    }
    Ok(())
}
