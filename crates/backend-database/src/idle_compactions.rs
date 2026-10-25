//! The roots that compact while idle, shortly before their prompt cache
//! expires.
//!
//! A root's answered request writes its row, or removes it when the request
//! fails a condition; its time's coming takes it. The rows outlast a restart,
//! so the next start keeps each time without reading any conversation's
//! database. A row goes with its conversation.

use std::time::Duration;

use demi_shared_types::Timestamp;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use rusqlite::OptionalExtension;

use super::StorageError;
use super::columns::decode;
use super::control::ControlService;

const TABLE: &str = "idle_compactions";

/// When a conversation's root compacts while idle, and the lifetime of the
/// cache that time comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleCompaction {
    pub due_at: Timestamp,
    pub lifetime: Duration,
}

impl ControlService {
    /// Writes when the conversation's root compacts while idle, or removes
    /// it with none. A conversation that is gone gets no row.
    pub async fn set_idle_compaction(
        &self,
        conversation: ConversationId,
        due: Option<IdleCompaction>,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            match due {
                Some(due) => {
                    let lifetime_ms = i64::try_from(due.lifetime.as_millis()).unwrap_or(i64::MAX);
                    connection.execute(
                        "INSERT INTO idle_compactions (conversation_id, due_at, lifetime_ms)
                           SELECT id, ?2, ?3 FROM conversations WHERE id = ?1
                           ON CONFLICT (conversation_id)
                           DO UPDATE SET due_at = excluded.due_at, lifetime_ms = excluded.lifetime_ms",
                        rusqlite::params![conversation.as_str(), due.due_at.as_millisecond(), lifetime_ms],
                    )?;
                }
                None => {
                    connection.execute(
                        "DELETE FROM idle_compactions WHERE conversation_id = ?1",
                        [conversation.as_str()],
                    )?;
                }
            }
            Ok(())
        })
        .await
    }

    /// The owners of the conversations that compact while idle.
    pub async fn idle_compaction_owners(&self) -> Result<Vec<UserId>, StorageError> {
        self.call(|connection, _| {
            let mut statement = connection.prepare(
                "SELECT DISTINCT conversations.user_id FROM idle_compactions
                   JOIN conversations ON conversations.id = idle_compactions.conversation_id
                   ORDER BY conversations.user_id",
            )?;
            let mut rows = statement.query([])?;
            let mut owners = Vec::new();
            while let Some(row) = rows.next()? {
                owners.push(decode("conversations", "user_id", UserId::try_from(row.get::<_, String>(0)?))?);
            }
            Ok(owners)
        })
        .await
    }

    /// `owner`'s conversations that compact while idle, with when.
    pub async fn idle_compactions(
        &self,
        owner: UserId,
    ) -> Result<Vec<(ConversationId, IdleCompaction)>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare(
                "SELECT idle_compactions.conversation_id, due_at, lifetime_ms FROM idle_compactions
                   JOIN conversations ON conversations.id = idle_compactions.conversation_id
                   WHERE conversations.user_id = ?1
                   ORDER BY due_at",
            )?;
            let mut rows = statement.query([owner.as_str()])?;
            let mut due = Vec::new();
            while let Some(row) = rows.next()? {
                let conversation = decode(TABLE, "conversation_id", ConversationId::try_from(row.get::<_, String>(0)?))?;
                due.push((conversation, read_row(row, 1)?));
            }
            Ok(due)
        })
        .await
    }

    /// When the conversation's root compacts while idle, when it does.
    pub async fn idle_compaction(&self, conversation: ConversationId) -> Result<Option<IdleCompaction>, StorageError> {
        self.call(move |connection, _| {
            connection
                .query_row(
                    "SELECT due_at, lifetime_ms FROM idle_compactions WHERE conversation_id = ?1",
                    [conversation.as_str()],
                    |row| Ok(read_row(row, 0)),
                )
                .optional()?
                .transpose()
        })
        .await
    }

    /// Removes the conversation's row as its time `due_at` comes; false when
    /// it is gone or holds a later time, which an answer written meanwhile
    /// set.
    pub async fn take_idle_compaction(
        &self,
        conversation: ConversationId,
        due_at: Timestamp,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let removed = connection.execute(
                "DELETE FROM idle_compactions WHERE conversation_id = ?1 AND due_at = ?2",
                rusqlite::params![conversation.as_str(), due_at.as_millisecond()],
            )?;
            Ok(removed == 1)
        })
        .await
    }
}

/// The time and lifetime of a row whose `due_at` is column `first`.
fn read_row(row: &rusqlite::Row<'_>, first: usize) -> Result<IdleCompaction, StorageError> {
    let due_at = decode(TABLE, "due_at", Timestamp::from_millisecond(row.get(first)?))?;
    let lifetime_ms: i64 = row.get(first + 1)?;
    let lifetime = decode(TABLE, "lifetime_ms", u64::try_from(lifetime_ms))?;
    Ok(IdleCompaction {
        due_at,
        lifetime: Duration::from_millis(lifetime),
    })
}
