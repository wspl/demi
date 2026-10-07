//! A conversation's deletion as the control database records it (`storage.md`
//! § Deleting a conversation): one transaction removes the conversation's
//! record with every record that belongs to it and records the deletion as
//! pending; the pending record goes once the conversation's database is
//! removed, and a start finishes each deletion it finds pending. The blobs
//! the conversation named are left to the collection of its owner's
//! namespace (`references`).

use demi_web_api_protocol::ids::{ConversationId, UserId};
use rusqlite::params;

use super::StorageError;
use super::columns::decode;
use super::control::ControlService;
use super::conversation_index::conversation_by_id;

/// A deletion whose conversation's record is gone and whose database may not
/// be yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDeletion {
    pub id: ConversationId,
    pub owner: UserId,
}

impl ControlService {
    /// Step 1 of a deletion: removes `owner`'s conversation `id` with its
    /// draft, work panel, attached hosts, permission requests and grants,
    /// and records the deletion as pending, in one transaction. Answers the
    /// id as the record spelled it, which names its database; none when the
    /// owner has no such conversation. The usage the conversation incurred
    /// stays in the ledger: it is the owner's account of what they used.
    pub async fn delete_conversation(
        &self,
        owner: UserId,
        id: ConversationId,
    ) -> Result<Option<ConversationId>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let Some(record) =
                conversation_by_id(&transaction, &id)?.filter(|record| record.owner == owner)
            else {
                return Ok(None);
            };
            let id = record.id;
            // The draft, the panel and the permission records go with the
            // record through their foreign keys; the attached hosts do not
            // cascade.
            transaction.execute(
                "DELETE FROM conversation_hosts WHERE conversation_id = ?1",
                [id.as_str()],
            )?;
            transaction.execute("DELETE FROM conversations WHERE id = ?1", [id.as_str()])?;
            transaction.execute(
                "INSERT INTO conversation_deletions (id, user_id, deleted_at) VALUES (?1, ?2, ?3)",
                params![id.as_str(), owner.as_str(), now.as_millisecond()],
            )?;
            transaction.commit()?;
            Ok(Some(id))
        })
        .await
    }

    /// The deletions not finished, oldest first.
    pub async fn pending_deletions(&self) -> Result<Vec<PendingDeletion>, StorageError> {
        self.call(|connection, _| {
            let mut statement = connection
                .prepare("SELECT id, user_id FROM conversation_deletions ORDER BY deleted_at, id")?;
            let mut rows = statement.query([])?;
            let mut pending = Vec::new();
            while let Some(row) = rows.next()? {
                pending.push(PendingDeletion {
                    id: decode(
                        "conversation_deletions",
                        "id",
                        ConversationId::try_from(row.get::<_, String>(0)?),
                    )?,
                    owner: decode(
                        "conversation_deletions",
                        "user_id",
                        UserId::try_from(row.get::<_, String>(1)?),
                    )?,
                });
            }
            Ok(pending)
        })
        .await
    }

    /// The last step of a deletion: its pending record goes.
    pub async fn finish_deletion(&self, id: ConversationId) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "DELETE FROM conversation_deletions WHERE id = ?1",
                [id.as_str()],
            )?;
            Ok(())
        })
        .await
    }
}
