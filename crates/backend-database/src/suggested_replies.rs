//! Each answer of a suggested replies request
//! (`crates/agent-server/src/suggested_replies.rs`), with what answered and
//! what the user did with it, which stays to measure the replies by. A
//! conversation offers its newest, so a restart of the backend offers it still.

use demi_shared_types::BlockId;
use demi_web_api_protocol::conversations::{SuggestedReplies, SuggestedReplyUse, SuggestedReplyUsed};
use demi_web_api_protocol::ids::ConversationId;
use rusqlite::{OptionalExtension as _, params};
use serde::Deserialize;

use super::StorageError;
use super::columns::{decode, json, to_json};
use super::control::ControlService;

const TABLE: &str = "conversation_suggested_replies";

/// The replies a row holds: each words to send, so none is empty.
#[derive(Deserialize, garde::Validate)]
#[garde(transparent)]
struct KeptReplies(#[garde(inner(length(chars, min = 1)))] Vec<String>);

/// One answer of a suggested replies request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnsweredReplies {
    /// The text block of the answer they follow.
    pub block_id: BlockId,
    /// The small model that answered.
    pub provider_id: String,
    pub model: String,
    /// A hash of the instruction it read.
    pub instruction_hash: String,
    /// Its whole answer, none when it had no text.
    pub response: Option<String>,
    /// What the page offers of it.
    pub replies: Vec<String>,
}

impl ControlService {
    /// What the newest answer of `conversation` offers, none before the
    /// first request answered.
    pub async fn suggested_replies(&self, conversation: ConversationId) -> Result<Option<SuggestedReplies>, StorageError> {
        self.call(move |connection, _| {
            let row = connection
                .prepare_cached("SELECT block_id, replies FROM conversation_suggested_replies WHERE conversation_id = ?1 ORDER BY id DESC LIMIT 1")?
                .query_row(params![conversation.as_str()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
                .optional()?;
            let Some((block_id, replies)) = row else {
                return Ok(None);
            };
            Ok(Some(SuggestedReplies {
                block_id: decode(TABLE, "block_id", BlockId::try_from(block_id))?,
                replies: json::<KeptReplies>(TABLE, "replies", &replies)?.0,
            }))
        })
        .await
    }

    /// Keeps `answered` as the newest answer of `conversation`; a conversation
    /// deleted meanwhile keeps nothing.
    pub async fn keep_suggested_replies(&self, conversation: ConversationId, answered: AnsweredReplies) -> Result<(), StorageError> {
        self.call(move |connection, now| {
            connection
                .prepare_cached(
                    "INSERT INTO conversation_suggested_replies
                       (conversation_id, block_id, answered_at, provider_id, model, instruction_hash, response, replies)
                     SELECT id, ?2, ?3, ?4, ?5, ?6, ?7, ?8 FROM conversations WHERE id = ?1",
                )?
                .execute(params![
                    conversation.as_str(),
                    answered.block_id.as_str(),
                    now.as_millisecond(),
                    answered.provider_id,
                    answered.model,
                    answered.instruction_hash,
                    answered.response,
                    to_json(&answered.replies),
                ])?;
            Ok(())
        })
        .await
    }

    /// Records the reply the user took of the newest answer that follows the
    /// block `used` names, in place of one taken before. A block or a place
    /// that holds no reply records nothing: this only measures.
    pub async fn suggested_reply_used(&self, conversation: ConversationId, used: SuggestedReplyUsed) -> Result<(), StorageError> {
        self.call(move |connection, now| {
            connection
                .prepare_cached(
                    "UPDATE conversation_suggested_replies SET used_reply = ?3, used_as = ?4, used_at = ?5
                     WHERE id = (SELECT MAX(id) FROM conversation_suggested_replies WHERE conversation_id = ?1 AND block_id = ?2)
                       AND ?3 < json_array_length(replies)",
                )?
                .execute(params![
                    conversation.as_str(),
                    used.block_id.as_str(),
                    used.reply,
                    match used.used_as {
                        SuggestedReplyUse::Sent => "sent",
                        SuggestedReplyUse::Edited => "edited",
                    },
                    now.as_millisecond(),
                ])?;
            Ok(())
        })
        .await
    }
}
