//! The roots whose running turn a graceful shutdown began to interrupt.
//!
//! The shutdown marks each such root before it aborts the turn, so a shutdown cut
//! short still leaves a mark for every root it began to interrupt. The next start
//! finds them here, without reading any conversation's database, and resumes each
//! once.
//!
//! A crash writes no mark. A mark goes with its conversation.

use demi_web_api_protocol::ids::ConversationId;

use super::StorageError;
use super::columns::decode;
use super::store::CoreStore;

impl CoreStore {
    /// Marks the conversation's root as one whose running turn the shutdown
    /// is about to interrupt. A conversation that is gone is not marked.
    pub async fn mark_interrupted_turn(&self, conversation: ConversationId) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "INSERT INTO interrupted_turns (conversation_id)
                   SELECT id FROM conversations WHERE id = ?1
                   ON CONFLICT (conversation_id) DO NOTHING",
                [conversation.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// `owner`'s marked conversations.
    pub async fn interrupted_turns(&self) -> Result<Vec<ConversationId>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare(
                "SELECT interrupted_turns.conversation_id FROM interrupted_turns
                   JOIN conversations ON conversations.id = interrupted_turns.conversation_id
                   ORDER BY interrupted_turns.conversation_id",
            )?;
            let mut rows = statement.query([])?;
            let mut marked = Vec::new();
            while let Some(row) = rows.next()? {
                marked.push(decode(
                    "interrupted_turns",
                    "conversation_id",
                    ConversationId::try_from(row.get::<_, String>(0)?),
                )?);
            }
            Ok(marked)
        })
        .await
    }

    /// Removes the conversation's mark, as its start takes it up; false
    /// when it is no longer there.
    pub async fn take_interrupted_turn(&self, conversation: ConversationId) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let removed = connection.execute(
                "DELETE FROM interrupted_turns WHERE conversation_id = ?1",
                [conversation.as_str()],
            )?;
            Ok(removed == 1)
        })
        .await
    }
}
