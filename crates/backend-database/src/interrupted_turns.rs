//! The roots whose running turn a graceful shutdown began to interrupt:
//! the shutdown marks each such root before it aborts the turn, so a
//! shutdown cut short still leaves a mark for every root it began to
//! interrupt, and the next start finds them here, without reading any
//! conversation's database, and resumes each once. A crash writes no mark.
//! A mark goes with its conversation.

use demi_web_api_protocol::ids::{ConversationId, UserId};

use super::StorageError;
use super::columns::decode;
use super::control::ControlService;

impl ControlService {
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

    /// The owners of the conversations a shutdown marked.
    pub async fn interrupted_turn_owners(&self) -> Result<Vec<UserId>, StorageError> {
        self.call(|connection, _| {
            let mut statement = connection.prepare(
                "SELECT DISTINCT conversations.user_id FROM interrupted_turns
                   JOIN conversations ON conversations.id = interrupted_turns.conversation_id
                   ORDER BY conversations.user_id",
            )?;
            let mut rows = statement.query([])?;
            let mut owners = Vec::new();
            while let Some(row) = rows.next()? {
                owners.push(decode(
                    "conversations",
                    "user_id",
                    UserId::try_from(row.get::<_, String>(0)?),
                )?);
            }
            Ok(owners)
        })
        .await
    }

    /// `owner`'s marked conversations.
    pub async fn interrupted_turns(&self, owner: UserId) -> Result<Vec<ConversationId>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare(
                "SELECT interrupted_turns.conversation_id FROM interrupted_turns
                   JOIN conversations ON conversations.id = interrupted_turns.conversation_id
                   WHERE conversations.user_id = ?1
                   ORDER BY interrupted_turns.conversation_id",
            )?;
            let mut rows = statement.query([owner.as_str()])?;
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
