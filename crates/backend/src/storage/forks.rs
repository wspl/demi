//! Fork creation attempts in the control store (`conversation-fork.md`
//! § Backend creation and retries): each reserves its destination's id with
//! its owner, source, text and creation metadata, and a control transaction
//! publishes the destination into the conversation index once its root is
//! committed in its own database. Publication is the public conversation
//! row; a reservation without one stays hidden.

use demi_core::{BlockId, ModelSelection, Timestamp};
use demi_web_api::conversations::ConversationTarget;
use demi_web_api::ids::{ConversationId, UserId};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::StorageError;
use super::columns::{decode, json, to_json};
use super::control::ControlService;
use super::conversation_index::{
    AttachedHostRecord, ConversationModel, ConversationRecord, NewConversation, TitleOrigin, conversation_by_id,
    insert_attached_host, insert_conversation,
};

/// One creation attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForkOperation {
    /// The destination's id, which the attempt reserves.
    pub(crate) id: ConversationId,
    pub(crate) owner: UserId,
    pub(crate) source: ConversationId,
    /// The completed text the history is kept through.
    pub(crate) block: BlockId,
    pub(crate) metadata: ForkMetadata,
}

/// What the destination is published with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ForkMetadata {
    #[garde(length(min = 1))]
    pub(crate) title: String,
    #[garde(dive)]
    pub(crate) target: ConversationTarget,
    /// The source's model selection, which the destination inherits.
    #[garde(dive)]
    pub(crate) model: ModelSelection,
    #[garde(skip)]
    pub(crate) created_at: Timestamp,
    #[garde(dive)]
    pub(crate) attached_hosts: Vec<AttachedHostRecord>,
}

impl ForkOperation {
    /// Whether `other` is this same creation attempt: its owner, source and
    /// text.
    pub(crate) fn same_attempt(&self, owner: &UserId, source: &ConversationId, block: &BlockId) -> bool {
        self.owner == *owner && same_id(&self.source, source) && self.block == *block
    }
}

/// Conversation ids compare without case.
fn same_id(one: &ConversationId, other: &ConversationId) -> bool {
    one.as_str().eq_ignore_ascii_case(other.as_str())
}

const OPERATION_COLUMNS: &str = "id, user_id, source_id, block_id, metadata";

impl ControlService {
    /// The creation attempt that reserved `id`, in whichever case it is
    /// spelled.
    pub(crate) async fn fork_operation(&self, id: ConversationId) -> Result<Option<ForkOperation>, StorageError> {
        self.call(move |connection, _| operation_by_id(connection, &id)).await
    }

    /// Reserves `operation`'s destination: the attempt that already holds
    /// it when it is this one, none when another attempt or a conversation
    /// holds the id.
    pub(crate) async fn reserve_fork(&self, operation: ForkOperation) -> Result<Option<ForkOperation>, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            if let Some(existing) = operation_by_id(&transaction, &operation.id)? {
                let same = existing.same_attempt(&operation.owner, &operation.source, &operation.block);
                return Ok(same.then_some(existing));
            }
            if conversation_by_id(&transaction, &operation.id)?.is_some() {
                return Ok(None);
            }
            transaction.execute(
                "INSERT INTO conversation_fork_operations (id, user_id, source_id, block_id, metadata)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    operation.id.as_str(),
                    operation.owner.as_str(),
                    operation.source.as_str(),
                    operation.block.as_str(),
                    to_json(&operation.metadata)
                ],
            )?;
            transaction.commit()?;
            Ok(Some(operation))
        })
        .await
    }

    /// The attempts whose destination is not published.
    pub(crate) async fn pending_forks(&self) -> Result<Vec<ForkOperation>, StorageError> {
        self.call(|connection, _| {
            let mut statement = connection.prepare(
                "SELECT f.id, f.user_id, f.source_id, f.block_id, f.metadata
                 FROM conversation_fork_operations f LEFT JOIN conversations c ON c.id = f.id
                 WHERE c.id IS NULL ORDER BY f.rowid",
            )?;
            let mut rows = statement.query([])?;
            let mut operations = Vec::new();
            while let Some(row) = rows.next()? {
                operations.push(operation_row(row)?);
            }
            Ok(operations)
        })
        .await
    }

    /// Publishes the destination `id` reserved: its conversation first in
    /// the owner's sidebar, with the title, target, model and attached hosts
    /// of the attempt, the title the user's. A destination published already
    /// is answered as it is. An attached host whose device is gone since is
    /// left out, as its revocation left the source.
    pub(crate) async fn publish_fork(&self, id: ConversationId) -> Result<ConversationRecord, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let operation = operation_by_id(&transaction, &id)?.ok_or_else(|| StorageError::Corrupt {
                table: "conversation_fork_operations",
                column: "id",
                reason: format!("no Fork reserved {id}"),
            })?;
            let metadata = &operation.metadata;
            let model = ConversationModel {
                provider: decode(
                    "conversation_fork_operations",
                    "metadata",
                    metadata.model.provider_id.as_str().try_into(),
                )?,
                model: metadata.model.model.id.clone(),
            };
            let inserted = insert_conversation(
                &transaction,
                &NewConversation {
                    id: &operation.id,
                    owner: &operation.owner,
                    title: &metadata.title,
                    origin: TitleOrigin::User,
                    target: &metadata.target,
                    model: Some(&model),
                    at: metadata.created_at,
                },
            )?;
            if inserted == 1 {
                for host in &metadata.attached_hosts {
                    let paired: bool = transaction.query_row(
                        "SELECT EXISTS (SELECT 1 FROM devices WHERE id = ?1)",
                        [host.device.as_str()],
                        |row| row.get(0),
                    )?;
                    if paired {
                        insert_attached_host(&transaction, &operation.id, host, metadata.created_at)?;
                    }
                }
            }
            let record = conversation_by_id(&transaction, &operation.id)?
                .filter(|record| record.owner == operation.owner)
                .ok_or_else(|| StorageError::Corrupt {
                    table: "conversations",
                    column: "user_id",
                    reason: format!("the Fork destination {id} belongs to another user"),
                })?;
            transaction.commit()?;
            Ok(record)
        })
        .await
    }
}

fn operation_by_id(connection: &Connection, id: &ConversationId) -> Result<Option<ForkOperation>, StorageError> {
    connection
        .query_row(
            &format!("SELECT {OPERATION_COLUMNS} FROM conversation_fork_operations WHERE id = ?1"),
            [id.as_str()],
            |row| Ok(operation_row(row)),
        )
        .optional()?
        .transpose()
}

/// A `conversation_fork_operations` row, read from its columns in
/// `OPERATION_COLUMNS`.
fn operation_row(row: &Row<'_>) -> Result<ForkOperation, StorageError> {
    const TABLE: &str = "conversation_fork_operations";
    let metadata: String = row.get("metadata")?;
    Ok(ForkOperation {
        id: decode(TABLE, "id", ConversationId::try_from(row.get::<_, String>("id")?))?,
        owner: decode(TABLE, "user_id", UserId::try_from(row.get::<_, String>("user_id")?))?,
        source: decode(TABLE, "source_id", ConversationId::try_from(row.get::<_, String>("source_id")?))?,
        block: decode(TABLE, "block_id", BlockId::try_from(row.get::<_, String>("block_id")?))?,
        metadata: json(TABLE, "metadata", &metadata)?,
    })
}
