//! Retention and a conversation's deletion as the control database records it.
//!
//! # Retention
//!
//! Demi keeps everything a conversation made for as long as the conversation
//! exists: its messages, the files the user sent in it, a tool result's images and
//! videos, a command's whole output and its media, edit copies and every other
//! blob. Nothing expires.
//!
//! Deleting the conversation removes it and the blobs no other record of its user
//! names. A blob a failed write leaves without a reference costs storage until the
//! next collection of its user's namespace, never correctness, since no row names
//! it.
//!
//! # Steps of a deletion
//!
//! A deletion goes in four steps:
//!
//! 1. One transaction of the control database removes the conversation's record
//!    and every record that belongs to it (its draft, work panel, attached hosts,
//!    permission requests and grants, shares), except the usage ledger's rows,
//!    which are the user's record of what was used. It records the deletion as
//!    pending, from which commit no request finds the conversation.
//! 2. The conversation's Host resources are released as archive releases them,
//!    and its tabs in the conversation browser are closed.
//! 3. Its database file and its rows in the search index are removed.
//! 4. The pending record is removed.
//!
//! A start that finds a pending deletion finishes it from the step it reached, so
//! a crash in between leaves nothing of the conversation behind.
//!
//! # What stays
//!
//! - Its project's files stay: they belong to the Host.
//! - The blobs the conversation named are left to the collection of its owner's
//!   namespace (`references`).
//! - There is no account deletion. When it comes it must also remove the account's
//!   blob namespace, `blobs/<userId>/`, which nothing else removes. A Cloud's
//!   disks stay the machine manager's.

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
    /// Step 1 of a deletion: removes `owner`'s conversation `id` with its draft,
    /// work panel, attached hosts, permission requests and grants, and records the
    /// deletion as pending, in one transaction.
    ///
    /// Answers the id as the record spelled it, which names its database; none
    /// when the owner has no such conversation. The usage the conversation incurred
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
