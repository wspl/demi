//! Fork creation attempts in the control store (`conversation-fork.md`
//! § Backend creation and retries): each reserves its destination's id with
//! its owner, source, text and creation metadata, and a control transaction
//! publishes the destination into the conversation index once its root is
//! committed in its own database. Publication is the public conversation
//! row; a reservation without one stays hidden.

use demi_shared_types::{BlockId, ModelSelection, Timestamp, WorkflowChoice};
use demi_web_api_protocol::conversations::ConversationTarget;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::StorageError;
use super::columns::{decode, json, to_json};
use super::control::ControlService;
use super::selection::{Selected, Selection, select};
use super::workspaces::WorkspaceRecord;
use super::conversation_index::{
    AttachedHostRecord, ConversationRecord, NewConversation, conversation_by_id,
    insert_attached_host, insert_conversation,
};

/// One creation attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkOperation {
    /// The destination's id, which the attempt reserves.
    pub id: ConversationId,
    pub owner: UserId,
    pub source: ConversationId,
    /// The completed text the history is kept through.
    pub block: BlockId,
    pub metadata: ForkMetadata,
}

/// What the destination is published with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForkMetadata {
    /// The source's title with " (Fork)"; null when the source's is the
    /// placeholder, which holds no words, so the destination's is too.
    #[serde(deserialize_with = "Option::deserialize")]
    #[garde(length(min = 1))]
    pub title: Option<String>,
    #[garde(dive)]
    pub target: ConversationTarget,
    /// The model selection the source's record held, which the destination
    /// inherits; none when the source had none.
    #[serde(deserialize_with = "Option::deserialize")]
    #[garde(dive)]
    pub model: Option<ModelSelection>,
    /// The source's Workflow menu choice, which the destination keeps as it
    /// keeps the model (`crates/backend-user-shard/src/conversation/workflows.rs`).
    #[garde(dive)]
    pub workflow: WorkflowChoice,
    #[garde(skip)]
    pub created_at: Timestamp,
    #[garde(dive)]
    pub attached_hosts: Vec<AttachedHostRecord>,
    /// The source's execution-context revision at the Fork, which the
    /// destination starts at, so the context blocks it copies are older
    /// than any it is announced.
    #[garde(skip)]
    pub context_version: u64,
}

impl ForkOperation {
    /// Whether `other` is this same creation attempt: its owner, source and
    /// text.
    pub fn same_attempt(&self, owner: &UserId, source: &ConversationId, block: &BlockId) -> bool {
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
    pub async fn fork_operation(
        &self,
        id: ConversationId,
    ) -> Result<Option<ForkOperation>, StorageError> {
        self.call(move |connection, _| operation_by_id(connection, &id))
            .await
    }

    /// Reserves `operation`'s destination: the attempt that already holds
    /// it when it is this one, none when another attempt or a conversation
    /// holds the id.
    pub async fn reserve_fork(
        &self,
        operation: ForkOperation,
    ) -> Result<Option<ForkOperation>, StorageError> {
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
    pub async fn pending_forks(&self) -> Result<Vec<ForkOperation>, StorageError> {
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
    /// the owner's sidebar, with the title, target, model, workflow and attached hosts
    /// of the attempt, the title the user's. A destination published already
    /// is answered as it is. An attached host whose device is gone since is
    /// left out, as its revocation left the source. Its directory is
    /// selected as a move's is (`sessions-and-targets.md` § Resolve a
    /// target), and the workspace that makes, if any, is answered with it;
    /// a directory that selects nothing any more, its device revoked or its
    /// project deleted since the Fork began, starts it on the Cloud in its
    /// own directory.
    pub async fn publish_fork(
        &self,
        id: ConversationId,
    ) -> Result<(ConversationRecord, Option<WorkspaceRecord>), StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let operation =
                operation_by_id(&transaction, &id)?.ok_or_else(|| StorageError::Corrupt {
                    table: "conversation_fork_operations",
                    column: "id",
                    reason: format!("no Fork reserved {id}"),
                })?;
            let metadata = &operation.metadata;
            let published = conversation_by_id(&transaction, &operation.id)?.is_some();
            let Selected { selection, made } = if published {
                Selected { selection: Selection::Cloud { path: None }, made: None }
            } else {
                select(&transaction, &operation.owner, &operation.id, &metadata.target, now)?
                    .unwrap_or(Selected { selection: Selection::Cloud { path: None }, made: None })
            };
            let inserted = insert_conversation(
                &transaction,
                &NewConversation {
                    id: &operation.id,
                    owner: &operation.owner,
                    title: metadata.title.as_deref(),
                    pinned: false,
                    target: &selection,
                    model: metadata.model.as_ref(),
                    workflow: &metadata.workflow,
                    context_version: metadata.context_version,
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
                        insert_attached_host(
                            &transaction,
                            &operation.id,
                            host,
                            metadata.created_at,
                        )?;
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
            Ok((record, made))
        })
        .await
    }
}

fn operation_by_id(
    connection: &Connection,
    id: &ConversationId,
) -> Result<Option<ForkOperation>, StorageError> {
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
        id: decode(
            TABLE,
            "id",
            ConversationId::try_from(row.get::<_, String>("id")?),
        )?,
        owner: decode(
            TABLE,
            "user_id",
            UserId::try_from(row.get::<_, String>("user_id")?),
        )?,
        source: decode(
            TABLE,
            "source_id",
            ConversationId::try_from(row.get::<_, String>("source_id")?),
        )?,
        block: decode(
            TABLE,
            "block_id",
            BlockId::try_from(row.get::<_, String>("block_id")?),
        )?,
        metadata: json(TABLE, "metadata", &metadata)?,
    })
}
