//! The replies each conversation's latest answer offers
//! (`crates/agent-server/src/suggested_replies.rs`), kept with the answer
//! they follow until a later answer's replace them, so a restart of the
//! backend offers them still.

use demi_shared_types::{BlockId, SuggestedReply};
use demi_web_api_protocol::conversations::SuggestedReplies;
use demi_web_api_protocol::ids::ConversationId;
use rusqlite::{OptionalExtension as _, params};

use super::StorageError;
use super::columns::{decode, json, to_json};
use super::control::ControlService;

const TABLE: &str = "conversation_suggested_replies";

impl ControlService {
    /// What the latest answer of `conversation` offers, none before the
    /// first request kept any.
    pub async fn suggested_replies(&self, conversation: ConversationId) -> Result<Option<SuggestedReplies>, StorageError> {
        self.call(move |connection, _| {
            let row = connection
                .prepare_cached("SELECT block_id, replies FROM conversation_suggested_replies WHERE conversation_id = ?1")?
                .query_row(params![conversation.as_str()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
                .optional()?;
            let Some((block_id, replies)) = row else {
                return Ok(None);
            };
            Ok(Some(SuggestedReplies {
                block_id: decode(TABLE, "block_id", BlockId::try_from(block_id))?,
                replies: json::<Vec<SuggestedReply>>(TABLE, "replies", &replies)?,
            }))
        })
        .await
    }

    /// Keeps `kept` as what the latest answer of `conversation` offers, in
    /// place of what an earlier answer offered; a conversation deleted
    /// meanwhile keeps nothing.
    pub async fn keep_suggested_replies(&self, conversation: ConversationId, kept: SuggestedReplies) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(
                    "INSERT INTO conversation_suggested_replies (conversation_id, block_id, replies)
                     SELECT id, ?2, ?3 FROM conversations WHERE id = ?1
                     ON CONFLICT (conversation_id) DO UPDATE SET block_id = excluded.block_id, replies = excluded.replies",
                )?
                .execute(params![conversation.as_str(), kept.block_id.as_str(), to_json(&kept.replies)])?;
            Ok(())
        })
        .await
    }
}
