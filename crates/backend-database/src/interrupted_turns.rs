//! The root turns a graceful shutdown interrupted (`failures-and-recovery.md`
//! § The unfinished turn): the shutdown marks each conversation whose
//! root's turn it ended with the `interrupted` record it wrote, and the next
//! start finds them here, without reading any conversation's database, and
//! resumes each once. A crash writes no mark: its record is written when the
//! conversation is next opened, and that turn waits for the user. A mark
//! goes with its conversation.

use demi_shared_types::BlockId;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use rusqlite::params;

use super::StorageError;
use super::columns::decode;
use super::control::ControlService;

/// A root turn a shutdown interrupted: its conversation, and the record
/// that shutdown wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptedTurn {
    pub conversation: ConversationId,
    pub error: BlockId,
}

impl ControlService {
    /// Marks the conversation `conversation`'s root turn as one a shutdown
    /// interrupted with the record `error`, in place of an earlier mark. A
    /// conversation that is gone is not marked.
    pub async fn mark_interrupted_turn(
        &self,
        conversation: ConversationId,
        error: BlockId,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "INSERT INTO interrupted_turns (conversation_id, error_block_id)
                   SELECT id, ?2 FROM conversations WHERE id = ?1
                   ON CONFLICT (conversation_id) DO UPDATE SET error_block_id = excluded.error_block_id",
                params![conversation.as_str(), error.as_str()],
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

    /// The marks of `owner`'s conversations.
    pub async fn interrupted_turns(&self, owner: UserId) -> Result<Vec<InterruptedTurn>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare(
                "SELECT interrupted_turns.conversation_id, interrupted_turns.error_block_id
                   FROM interrupted_turns
                   JOIN conversations ON conversations.id = interrupted_turns.conversation_id
                   WHERE conversations.user_id = ?1
                   ORDER BY interrupted_turns.conversation_id",
            )?;
            let mut rows = statement.query([owner.as_str()])?;
            let mut turns = Vec::new();
            while let Some(row) = rows.next()? {
                turns.push(InterruptedTurn {
                    conversation: decode(
                        "interrupted_turns",
                        "conversation_id",
                        ConversationId::try_from(row.get::<_, String>(0)?),
                    )?,
                    error: decode(
                        "interrupted_turns",
                        "error_block_id",
                        BlockId::try_from(row.get::<_, String>(1)?),
                    )?,
                });
            }
            Ok(turns)
        })
        .await
    }

    /// Removes the mark `turn`, as its start takes it up; false when it is
    /// no longer there, as when a later shutdown marked another record.
    pub async fn take_interrupted_turn(&self, turn: InterruptedTurn) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let removed = connection.execute(
                "DELETE FROM interrupted_turns WHERE conversation_id = ?1 AND error_block_id = ?2",
                params![turn.conversation.as_str(), turn.error.as_str()],
            )?;
            Ok(removed == 1)
        })
        .await
    }
}
