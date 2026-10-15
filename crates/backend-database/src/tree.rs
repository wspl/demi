//! The tree store over a conversation's database (`runtime.md` § Tree store,
//! `storage.md` § Conversation state and transactions): a node's record is
//! its `nodes` row, and its checkpoint is that row's state with the node's
//! `blocks` rows. Creating, saving, closing, reopening and deleting a node
//! are each one transaction on the conversation's writer connection, where
//! rows are also serialized and decoded. Every row read is decoded and
//! checked, and corrupt data stops the read.
//!
//! Blocks hold their media by reference, and a checkpoint that holds media
//! bytes is refused; each node's session reaches the conversation owner's
//! blob namespace through its store (`storage.md` § Attachment and
//! transcript media).
//!
//! The same readings serve what the web app reads without a live session: a
//! conversation's summary facts and its history, on a read-only connection,
//! which keep the media references.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use demi_agent_store::{AgentTreeStore, RunningCommand, SessionStore, StoredCommand, StoredOutput};
use demi_agent_store::{
    Checkpoint, CheckpointState, CheckpointUpdate, ChildDirectory, ClosePhase, NodeClose, NodeRecord, Placement, StoreError,
    media::BlobStore,
};
use demi_backend_remote_host::decode_output;
use demi_conversation_socket_protocol::{PageAt, light_bytes, page, requests};
use demi_web_api_protocol::conversations::{CompactionSummary, TranscriptQuery};
use demi_shared_types::{
    Block, BlockId, CommandEnd, CommandId, CompletionId, INSTRUCTIONS_SOURCE, InstructionEntry,
    ModelSelection, NodeId, QueuedMessage, Sequence, SessionPhase, Timestamp, ToolCallBlock, is_blank,
};
use futures_util::future::LocalBoxFuture;
use demi_web_api_protocol::ids::ShareToken;
use demi_agent_store::media::block_blobs;
use crate::shares::{self, Rewritten};
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params, params_from_iter};

use super::StorageError;
use super::columns::{count, decode, instant, json, to_json};
use super::command_outputs::{self, OutputRow};
use super::conversations::ConversationDb;
use super::{conversation_attachments, running, sequences};

/// What a tree store tells after each commit that writes a node's
/// checkpoint, closes, reopens or deletes a node, with that node: a
/// conversation's summary reads its root's checkpoint.
pub type Saved = Rc<dyn Fn(&NodeId)>;

/// One conversation's agent tree in its database, with its owner's blobs.
pub struct SqliteTreeStore {
    db: ConversationDb,
    blobs: Arc<dyn BlobStore>,
    saved: Saved,
}

impl SqliteTreeStore {
    pub fn new(db: ConversationDb, blobs: Arc<dyn BlobStore>, saved: Saved) -> Self {
        Self { db, blobs, saved }
    }
}

/// One node's checkpoint in its conversation's database.
struct SqliteSessionStore {
    db: ConversationDb,
    blobs: Arc<dyn BlobStore>,
    node: NodeId,
    saved: Saved,
}

/// A store failure as the agent sees it: corrupt data, or an operation that
/// failed.
fn store_error(error: StorageError) -> StoreError {
    match error {
        StorageError::Corrupt { .. } => StoreError::Corrupt(error.to_string()),
        other => StoreError::Failed(other.to_string()),
    }
}

fn missing(node: &NodeId) -> StoreError {
    StoreError::Failed(format!("no node {node}"))
}

impl AgentTreeStore for SqliteTreeStore {
    fn node<'a>(
        &'a self,
        id: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<Option<NodeRecord>, StoreError>> {
        let id = id.clone();
        Box::pin(async move {
            self.db
                .call(move |connection| node_by_id(connection, &id))
                .await
                .map_err(store_error)
        })
    }

    fn children<'a>(
        &'a self,
        parent: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<Vec<NodeRecord>, StoreError>> {
        let parent = parent.clone();
        Box::pin(async move {
            self.db
                .call(move |connection| children_of(connection, &parent))
                .await
                .map_err(store_error)
        })
    }

    fn subagents(&self) -> LocalBoxFuture<'_, Result<Vec<(NodeRecord, ModelSelection)>, StoreError>> {
        Box::pin(async move { self.db.call(|connection| subagents(connection)).await.map_err(store_error) })
    }

    fn create_node(
        &self,
        record: NodeRecord,
        initial: CheckpointUpdate,
    ) -> LocalBoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            let completions = initial.carried_completions()?;
            let node = record.id.clone();
            self.db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    if node_by_id(&transaction, &record.id)?.is_some() {
                        return Ok(Err(StoreError::Failed(format!("node {} already exists", record.id))));
                    }
                    let (phase, closed_at, result, failure) = close_columns(record.closed.as_ref());
                    transaction.execute(
                        "INSERT INTO nodes (id, number, parent_id, description, profile, instructions, round, started_at,
                           can_spawn, closed_phase, closed_at, result, failure, delivered, state, block_count,
                           output_revision, forked_from, cwd, cwd_device, cwd_under, context_limit)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, 0, 0, ?16, ?17, ?18, ?19, ?20)",
                        params![
                            record.id.as_str(),
                            integer(record.number),
                            record.parent.as_ref().map(NodeId::as_str),
                            record.description,
                            record.profile,
                            record.instructions,
                            integer(record.round),
                            record.started_at.as_millisecond(),
                            record.can_spawn_subagents,
                            phase,
                            closed_at,
                            result,
                            failure,
                            record.delivered,
                            to_json(&initial.state),
                            record.forked_from.map(integer),
                            record.cwd.as_ref().map(|own| &own.cwd),
                            record.cwd.as_ref().and_then(|own| own.under.device.as_ref()),
                            record.cwd.as_ref().map(|own| &own.under.directory),
                            record.context_limit,
                        ],
                    )?;
                    if let Err(refused) = write_checkpoint(&transaction, &record.id, &initial, &completions)? {
                        return Ok(Err(refused));
                    }
                    transaction.commit()?;
                    Ok(Ok(()))
                })
                .await
                .map_err(store_error)??;
            (self.saved)(&node);
            Ok(())
        })
    }

    fn session_store(&self, id: &NodeId) -> Rc<dyn SessionStore> {
        Rc::new(SqliteSessionStore {
            db: self.db.clone(),
            blobs: self.blobs.clone(),
            node: id.clone(),
            saved: self.saved.clone(),
        })
    }

    fn close_node<'a>(
        &'a self,
        id: &'a NodeId,
        close: NodeClose,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        let node = id.clone();
        Box::pin(async move {
            let (phase, closed_at, result, failure) = close_columns(Some(&close));
            let changed = self
                .db
                .call(move |connection| {
                    let changed = connection.execute(
                        "UPDATE nodes SET closed_phase = ?2, closed_at = ?3, result = ?4, failure = ?5, delivered = 0
                         WHERE id = ?1",
                        params![node.as_str(), phase, closed_at, result, failure],
                    )?;
                    Ok(changed)
                })
                .await
                .map_err(store_error)?;
            if changed == 0 {
                return Err(missing(id));
            }
            (self.saved)(id);
            Ok(())
        })
    }

    fn reopen_node<'a>(
        &'a self,
        id: &'a NodeId,
        round: u64,
        started_at: Timestamp,
        message: QueuedMessage,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        let node = id.clone();
        Box::pin(async move {
            let found = self
                .db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    let Some(state) = node_state(&transaction, &node)? else {
                        return Ok(false);
                    };
                    let state = CheckpointState {
                        queue: vec![message],
                        ..state
                    };
                    transaction.execute(
                        "UPDATE nodes SET round = ?2, started_at = ?3, closed_phase = NULL, closed_at = NULL,
                           result = NULL, failure = NULL, delivered = 0, state = ?4
                         WHERE id = ?1",
                        params![node.as_str(), integer(round), started_at.as_millisecond(), to_json(&state)],
                    )?;
                    transaction.commit()?;
                    Ok(true)
                })
                .await
                .map_err(store_error)?;
            if !found {
                return Err(missing(id));
            }
            (self.saved)(id);
            Ok(())
        })
    }

    fn set_cwd<'a>(
        &'a self,
        id: &'a NodeId,
        cwd: &'a ChildDirectory,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        let node = id.clone();
        let cwd = cwd.clone();
        Box::pin(async move {
            let changed = self
                .db
                .call(move |connection| {
                    Ok(connection.execute(
                        "UPDATE nodes SET cwd = ?2, cwd_device = ?3, cwd_under = ?4 WHERE id = ?1 AND parent_id IS NOT NULL",
                        params![node.as_str(), cwd.cwd, cwd.under.device, cwd.under.directory],
                    )?)
                })
                .await
                .map_err(store_error)?;
            if changed == 0 {
                return Err(missing(id));
            }
            (self.saved)(id);
            Ok(())
        })
    }

    fn mark_delivered<'a>(
        &'a self,
        id: &'a NodeId,
        round: u64,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        let node = id.clone();
        Box::pin(async move {
            let found = self
                .db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    if node_by_id(&transaction, &node)?.is_none() {
                        return Ok(false);
                    }
                    transaction.execute(
                        "UPDATE nodes SET delivered = 1 WHERE id = ?1 AND round = ?2",
                        params![node.as_str(), integer(round)],
                    )?;
                    transaction.commit()?;
                    Ok(true)
                })
                .await
                .map_err(store_error)?;
            if !found {
                return Err(missing(id));
            }
            Ok(())
        })
    }

    fn delete_node<'a>(&'a self, id: &'a NodeId) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        let node = id.clone();
        Box::pin(async move {
            // The node's descendants and every row of theirs go with it
            // through the cascades.
            self.db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    shares::keep_rewritten(&transaction, Rewritten::Subtree(&node))?;
                    transaction.execute("DELETE FROM nodes WHERE id = ?1", [node.as_str()])?;
                    transaction.commit()?;
                    Ok(())
                })
                .await
                .map_err(store_error)?;
            (self.saved)(id);
            Ok(())
        })
    }

    fn next_number(&self, sequence: Sequence) -> LocalBoxFuture<'_, Result<u64, StoreError>> {
        Box::pin(async move {
            self.db
                .call(move |connection| sequences::next(connection, sequence))
                .await
                .map_err(store_error)
        })
    }

    fn command_end<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<Option<CommandEnd>, StoreError>> {
        Box::pin(async move {
            let wanted = command.clone();
            let row = self
                .db
                .read(move |connection| command_outputs::read(connection, &wanted))
                .await
                .map_err(store_error)?
                .flatten();
            Ok(row.map(|row| row.end))
        })
    }

    fn running_commands<'a>(
        &'a self,
        node: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<Vec<RunningCommand>, StoreError>> {
        Box::pin(async move {
            let wanted = node.clone();
            let rows = self
                .db
                .read(move |connection| running::of_node(connection, &wanted))
                .await
                .map_err(store_error)?
                .unwrap_or_default();
            Ok(rows
                .into_iter()
                .map(|mut row| RunningCommand {
                    place: row.places.remove(node).unwrap_or_default(),
                    command: row.command,
                    job: row.job,
                    tool_use_id: row.tool_use_id,
                    title: row.title,
                    started: row.started,
                })
                .collect())
        })
    }

    fn record_place<'a>(
        &'a self,
        command: &'a CommandId,
        node: &'a NodeId,
        seen: demi_host_interface::Seen,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let (command, node) = (command.clone(), node.clone());
            self.db
                .call(move |connection| running::set_place(connection, &command, &node, seen))
                .await
                .map_err(store_error)
        })
    }

    fn place<'a>(
        &'a self,
        command: &'a CommandId,
        node: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<Option<demi_host_interface::Seen>, StoreError>> {
        Box::pin(async move {
            let (command, node) = (command.clone(), node.clone());
            Ok(self
                .db
                .read(move |connection| running::place(connection, &command, &node))
                .await
                .map_err(store_error)?
                .flatten())
        })
    }

    /// Reads the command's row on a read-only connection, then its blob,
    /// which it decodes on the blocking pool: a stored output takes up to
    /// 16 MiB.
    fn command_output<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<Option<StoredCommand>, StoreError>> {
        Box::pin(async move {
            let wanted = command.clone();
            let row = self
                .db
                .read(move |connection| command_outputs::read(connection, &wanted))
                .await
                .map_err(store_error)?
                .flatten();
            let Some(row) = row else {
                return Ok(None);
            };
            let (blob, missing) = match row.output {
                OutputRow::Stored { blob, missing } => (blob, missing),
                OutputRow::NotStored(reason) => {
                    return Ok(Some(StoredCommand {
                        title: row.title,
                        end: row.end,
                        output: StoredOutput::NotStored(reason),
                    }));
                }
            };
            let bytes = self.blobs.get(&blob).await?.ok_or_else(|| {
                StoreError::Failed(format!(
                    "the blob {blob} of the output of {command} is missing"
                ))
            })?;
            let output =
                tokio::task::spawn_blocking(move || decode_output(bytes.as_bytes(), missing))
                    .await
                    .map_err(|error| StoreError::Failed(error.to_string()))?
                    .map_err(|error| {
                        StoreError::Corrupt(format!(
                            "the output of {command} does not decode: {error}"
                        ))
                    })?;
            Ok(Some(StoredCommand {
                title: row.title,
                end: row.end,
                output: StoredOutput::Stored { output },
            }))
        })
    }
}

impl SessionStore for SqliteSessionStore {
    fn save(&self, update: CheckpointUpdate) -> LocalBoxFuture<'_, Result<(), StoreError>> {
        let node = self.node.clone();
        Box::pin(async move {
            let completions = update.carried_completions()?;
            let commit = self.db.commit_point();
            self.db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    if let Err(refused) =
                        write_checkpoint(&transaction, &node, &update, &completions)?
                    {
                        return Ok(Err(refused));
                    }
                    commit.commit(transaction)?;
                    Ok(Ok(()))
                })
                .await
                .map_err(store_error)??;
            (self.saved)(&self.node);
            Ok(())
        })
    }

    fn load(&self) -> LocalBoxFuture<'_, Result<Option<Checkpoint>, StoreError>> {
        let node = self.node.clone();
        Box::pin(async move {
            self.db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    checkpoint(&transaction, &node)
                })
                .await
                .map_err(store_error)
        })
    }

    fn blobs(&self) -> &dyn BlobStore {
        &*self.blobs
    }
}

/// Writes one save of `node` in `transaction`: the changed block rows with
/// the blobs each names, the rows past the new end gone with theirs, the
/// state row, and
/// the child completions it carries marked delivered. Changed output
/// advances the node's output revision; input alone does not. A refusal
/// leaves the transaction uncommitted.
fn write_checkpoint(
    transaction: &Transaction<'_>,
    node: &NodeId,
    update: &CheckpointUpdate,
    completions: &[CompletionId],
) -> Result<Result<(), StoreError>, StorageError> {
    let mut write = transaction.prepare_cached(
        "INSERT INTO blocks (node_id, idx, block) VALUES (?1, ?2, ?3)
         ON CONFLICT (node_id, idx) DO UPDATE SET block = excluded.block",
    )?;
    for (index, block) in &update.changed_blocks {
        let index = count(*index);
        shares::keep_rewritten(transaction, Rewritten::Row(node, index))?;
        write.execute(params![node.as_str(), index, to_json(block)])?;
        name_block(transaction, node.as_str(), index, block)?;
    }
    let block_count = count(update.block_count);
    shares::keep_rewritten(transaction, Rewritten::From(node, block_count))?;
    transaction
        .prepare_cached("DELETE FROM blocks WHERE node_id = ?1 AND idx >= ?2")?
        .execute(params![node.as_str(), block_count])?;
    let output = update
        .changed_blocks
        .iter()
        .any(|(_, block)| is_output(block));
    // The old block count is the column's value before the update: rows
    // gone are a rewrite of the output.
    let changed = transaction.execute(
        "UPDATE nodes SET state = ?2, block_count = ?3,
           output_revision = output_revision + (CASE WHEN block_count > ?3 OR ?4 THEN 1 ELSE 0 END)
         WHERE id = ?1",
        params![node.as_str(), to_json(&update.state), block_count, output],
    )?;
    if changed == 0 {
        return Ok(Err(missing(node)));
    }
    for round in completions {
        transaction.execute(
            "UPDATE nodes SET delivered = 1 WHERE id = ?1 AND parent_id = ?2 AND round = ?3",
            params![round.child.as_str(), node.as_str(), integer(round.round)],
        )?;
    }
    Ok(Ok(()))
}

/// Lists what the block at `index` of `node` names, in place of what its
/// row named: the blobs, in `block_blobs`, and for a text the attachments,
/// in `block_attachments`, so neither is found by reading block text.
pub(crate) fn name_block(
    transaction: &Transaction<'_>,
    node: &str,
    index: i64,
    block: &Block,
) -> Result<(), rusqlite::Error> {
    transaction
        .prepare_cached("DELETE FROM block_blobs WHERE node_id = ?1 AND idx = ?2")?
        .execute(params![node, index])?;
    transaction
        .prepare_cached("DELETE FROM block_attachments WHERE node_id = ?1 AND idx = ?2")?
        .execute(params![node, index])?;
    let mut blob = transaction
        .prepare_cached("INSERT OR IGNORE INTO block_blobs (node_id, idx, blob) VALUES (?1, ?2, ?3)")?;
    for named in block_blobs(block) {
        blob.execute(params![node, index, named.as_str()])?;
    }
    if let Block::Text(text) = block
        && text.text.contains("attachment:")
    {
        let mut attachment = transaction.prepare_cached(
            "INSERT OR IGNORE INTO block_attachments (node_id, idx, attachment) VALUES (?1, ?2, ?3)",
        )?;
        for number in shares::named_attachments(&text.text) {
            let number = i64::try_from(number.0).expect("a parsed attachment number fits the column");
            attachment.execute(params![node, index, number])?;
        }
    }
    Ok(())
}

/// Whether a block is output, which the user has not seen until the page
/// shows it: anything but the input blocks, the user's messages and steers
/// and the inputs the session and the tree write.
fn is_output(block: &Block) -> bool {
    !matches!(
        block,
        Block::User(_)
            | Block::Context(_)
            | Block::Wakeup(_)
            | Block::Steer(_)
            | Block::AgentMessage(_)
            | Block::Resume(_)
    )
}


/// A node's checkpoint: its state row and every block row below its block
/// count; none when the node does not exist.
fn checkpoint(connection: &Connection, node: &NodeId) -> Result<Option<Checkpoint>, StorageError> {
    let row: Option<(String, i64)> = connection
        .query_row(
            "SELECT state, block_count FROM nodes WHERE id = ?1",
            [node.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((state, block_count)) = row else {
        return Ok(None);
    };
    Ok(Some(Checkpoint {
        state: json("nodes", "state", &state)?,
        transcript: blocks_of(connection, node, block_count)?,
    }))
}

/// A node's blocks: exactly one row for each index below its block count.
pub(crate) fn blocks_of(
    connection: &Connection,
    node: &NodeId,
    block_count: i64,
) -> Result<Vec<Block>, StorageError> {
    let mut statement = connection
        .prepare("SELECT idx, block FROM blocks WHERE node_id = ?1 AND idx < ?2 ORDER BY idx")?;
    let mut rows = statement.query(params![node.as_str(), block_count])?;
    let mut blocks = Vec::new();
    while let Some(row) = rows.next()? {
        let index: i64 = row.get(0)?;
        if index != count(blocks.len()) {
            return Err(gap(node, blocks.len()));
        }
        let text: String = row.get(1)?;
        blocks.push(json("blocks", "block", &text)?);
    }
    if count(blocks.len()) != block_count {
        return Err(gap(node, blocks.len()));
    }
    Ok(blocks)
}

fn gap(node: &NodeId, index: usize) -> StorageError {
    StorageError::Corrupt {
        table: "blocks",
        column: "idx",
        reason: format!("node {node} has no block row {index}"),
    }
}

/// The state row of `node`, decoded.
fn node_state(
    connection: &Connection,
    node: &NodeId,
) -> Result<Option<CheckpointState>, StorageError> {
    let state: Option<String> = connection
        .query_row(
            "SELECT state FROM nodes WHERE id = ?1",
            [node.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    state
        .map(|state| json("nodes", "state", &state))
        .transpose()
}

const NODE_COLUMNS: &str =
    "id, number, parent_id, description, profile, instructions, round, started_at, can_spawn,
     closed_phase, closed_at, result, failure, delivered, forked_from, cwd, cwd_device, cwd_under,
     context_limit";

fn node_by_id(connection: &Connection, id: &NodeId) -> Result<Option<NodeRecord>, StorageError> {
    let mut statement =
        connection.prepare(&format!("SELECT {NODE_COLUMNS} FROM nodes WHERE id = ?1"))?;
    let mut rows = statement.query([id.as_str()])?;
    rows.next()?.map(node_row).transpose()
}

/// A node's direct children in spawn order, the order of their numbers.
fn children_of(connection: &Connection, parent: &NodeId) -> Result<Vec<NodeRecord>, StorageError> {
    let mut statement = connection.prepare(&format!(
        "SELECT {NODE_COLUMNS} FROM nodes WHERE parent_id = ?1 ORDER BY number"
    ))?;
    let mut rows = statement.query([parent.as_str()])?;
    let mut children = Vec::new();
    while let Some(row) = rows.next()? {
        children.push(node_row(row)?);
    }
    Ok(children)
}

/// A `nodes` row, read from its columns in `NODE_COLUMNS`.
fn node_row(row: &Row<'_>) -> Result<NodeRecord, StorageError> {
    const TABLE: &str = "nodes";
    let parent: Option<String> = row.get("parent_id")?;
    let phase: Option<String> = row.get("closed_phase")?;
    let closed = match phase {
        None => None,
        Some(phase) => {
            let phase = match phase.as_str() {
                "completed" => ClosePhase::Completed {
                    result: required(row, "result")?,
                },
                "aborted" => ClosePhase::Aborted,
                "error" => ClosePhase::Error {
                    failure: required(row, "failure")?,
                },
                other => {
                    return Err(StorageError::Corrupt {
                        table: TABLE,
                        column: "closed_phase",
                        reason: format!("unknown close phase {other}"),
                    });
                }
            };
            let at: Option<i64> = row.get("closed_at")?;
            let at = at.ok_or_else(|| StorageError::Corrupt {
                table: TABLE,
                column: "closed_at",
                reason: "a closed node has no close time".into(),
            })?;
            Some(NodeClose {
                phase,
                at: decode(TABLE, "closed_at", Timestamp::from_millisecond(at))?,
            })
        }
    };
    // A child works in a directory of its own; the root in the
    // conversation's, which no row holds.
    let device: Option<String> = row.get("cwd_device")?;
    let cwd = match (row.get::<_, Option<String>>("cwd")?, row.get::<_, Option<String>>("cwd_under")?) {
        (Some(cwd), Some(directory)) => Some(ChildDirectory {
            cwd,
            under: Placement { device, directory },
        }),
        (None, None) if device.is_none() => None,
        _ => {
            return Err(StorageError::Corrupt {
                table: TABLE,
                column: "cwd",
                reason: "a working directory without where the conversation worked".into(),
            });
        }
    };
    if cwd.is_some() != parent.is_some() {
        return Err(StorageError::Corrupt {
            table: TABLE,
            column: "cwd",
            reason: "a child has a working directory, and the root none".into(),
        });
    }
    Ok(NodeRecord {
        id: decode(TABLE, "id", NodeId::try_from(row.get::<_, String>("id")?))?,
        number: decode(TABLE, "number", u64::try_from(row.get::<_, i64>("number")?))?,
        parent: parent
            .map(|parent| decode(TABLE, "parent_id", NodeId::try_from(parent)))
            .transpose()?,
        description: row.get("description")?,
        profile: row.get("profile")?,
        instructions: row.get("instructions")?,
        context_limit: row
            .get::<_, Option<i64>>("context_limit")?
            .map(|tokens| decode(TABLE, "context_limit", u32::try_from(tokens)))
            .transpose()?,
        round: decode(TABLE, "round", u64::try_from(row.get::<_, i64>("round")?))?,
        started_at: instant(row, TABLE, "started_at")?,
        can_spawn_subagents: row.get("can_spawn")?,
        closed,
        delivered: row.get("delivered")?,
        forked_from: row
            .get::<_, Option<i64>>("forked_from")?
            .map(|number| decode(TABLE, "forked_from", u64::try_from(number)))
            .transpose()?,
        cwd,
    })
}

/// A text column its row's close phase requires.
fn required(row: &Row<'_>, column: &'static str) -> Result<String, StorageError> {
    let value: Option<String> = row.get(column)?;
    value.ok_or_else(|| StorageError::Corrupt {
        table: "nodes",
        column,
        reason: "the close phase requires it".into(),
    })
}

/// A close as its columns: the phase, the time, and the result or failure
/// the phase carries.
fn close_columns(
    close: Option<&NodeClose>,
) -> (
    Option<&'static str>,
    Option<i64>,
    Option<String>,
    Option<String>,
) {
    let Some(close) = close else {
        return (None, None, None, None);
    };
    let at = Some(close.at.as_millisecond());
    match &close.phase {
        ClosePhase::Completed { result } => (Some("completed"), at, Some(result.clone()), None),
        ClosePhase::Aborted => (Some("aborted"), at, None, None),
        ClosePhase::Error { failure } => (Some("error"), at, None, Some(failure.clone())),
    }
}

/// A count as the INTEGER column holds it: an agent's number counts spawns,
/// a round resumes and a revision saves, so none comes near `i64::MAX`.
fn integer(value: u64) -> i64 {
    i64::try_from(value).expect("a count fits the column")
}

/// What a conversation's summary is built from (`storage.md` § Conversation
/// state and transactions): the root's phase and output revision, the kind
/// of its latest terminal block and its latest ended turn, read without
/// loading the transcript, and the number of its latest attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryFacts {
    pub phase: SessionPhase,
    pub revision: u64,
    pub last: Option<Terminal>,
    pub last_turn: Option<EndedTurn>,
    pub last_attachment: u64,
}

impl SummaryFacts {
    /// The facts of a conversation that has no tree yet: idle, revision 0,
    /// no attachment.
    pub const EMPTY: Self = Self {
        phase: SessionPhase::Idle,
        revision: 0,
        last: None,
        last_turn: None,
        last_attachment: 0,
    };
}

/// How the root's latest finished request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terminal {
    Response,
    Error,
    Abort,
}

/// The root's latest ended turn (`web-api.md` § Sidebar mutations and read
/// state).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndedTurn {
    /// The terminal block that ended it: a turn that a Resume continues
    /// ends anew, with another block.
    pub id: BlockId,
    pub outcome: Terminal,
    /// The Markdown of its last text that is not blank; none when it gave
    /// none.
    pub answer: Option<String>,
}

/// One block row as the summary walks the transcript back: its kind, its
/// turn when it is a block of a turn's input, whether it is a terminal block
/// that ends a turn, and its text when it is a text block.
struct WalkRow {
    index: i64,
    kind: String,
    turn: Option<String>,
    ends_turn: bool,
    text: Option<String>,
}

/// What the walk found: the kind of the latest terminal block, and the row
/// of the block that ended the latest ended turn with that turn's last text
/// that is not blank.
struct Walk {
    last: Option<Terminal>,
    end: Option<i64>,
    answer: Option<String>,
}

/// How far the walk has read the ended turn back from its end.
#[derive(Default)]
struct EndSearch {
    end: Option<i64>,
    /// The ended turn, once the walk reached one of its input blocks.
    turn: Option<String>,
    /// The last text before the input block the walk reached last: the
    /// ended turn's when another of its input blocks comes before it.
    candidate: Option<String>,
    answer: Option<String>,
}

impl EndSearch {
    /// Reads the next row back; true once it knows the end and the answer.
    fn step(&mut self, row: WalkRow) -> bool {
        if self.end.is_none() {
            if row.ends_turn {
                self.end = Some(row.index);
            }
            return false;
        }
        match (row.turn, row.text) {
            (Some(input), _) => {
                if self.turn.as_ref().is_some_and(|turn| *turn != input) {
                    // The input of an earlier turn: the ended one gave no text.
                    return true;
                }
                if self.candidate.is_some() {
                    self.answer = self.candidate.take();
                    return true;
                }
                self.turn = Some(input);
                false
            }
            (None, Some(text)) if !is_blank(&text) => {
                if self.turn.is_none() {
                    // After the turn's last input block, so the turn's.
                    self.answer = Some(text);
                    return true;
                }
                self.candidate.get_or_insert(text);
                false
            }
            (None, _) => false,
        }
    }
}

/// The summary facts of the tree `connection` holds; the empty facts when it
/// has no root yet.
pub fn summary(connection: &Connection) -> Result<SummaryFacts, StorageError> {
    let root: Option<(String, String, i64, i64)> = connection
        .query_row(
            "SELECT id, state, block_count, output_revision FROM nodes WHERE parent_id IS NULL",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let last_attachment = conversation_attachments::last(connection)?;
    let Some((id, state, block_count, revision)) = root else {
        return Ok(SummaryFacts {
            last_attachment,
            ..SummaryFacts::EMPTY
        });
    };
    let state: CheckpointState = json("nodes", "state", &state)?;
    // A failed `compact` ends no turn, and a stop that a Resume continued
    // ended the turn only until the Resume.
    let mut rows = connection.prepare_cached(
        "SELECT idx, json_extract(block, '$.type'), json_extract(block, '$.turnId'),
                json_extract(block, '$.type') IN ('response', 'error', 'abort')
                  AND coalesce(json_extract(block, '$.outsideTurn'), 0) = 0
                  AND coalesce(json_extract(block, '$.isResumed'), 0) = 0,
                CASE json_extract(block, '$.type') WHEN 'text' THEN json_extract(block, '$.text') END
           FROM blocks WHERE node_id = ?1 AND idx < ?2 ORDER BY idx DESC",
    )?;
    let rows = rows.query_map(params![id, block_count], |row| {
        Ok(WalkRow {
            index: row.get(0)?,
            kind: row.get(1)?,
            turn: row.get(2)?,
            ends_turn: row.get(3)?,
            text: row.get(4)?,
        })
    })?;
    let walk = walk(rows.map(|row| row.map_err(StorageError::from)), state.phase)?;
    let last_turn = match walk.end {
        Some(end) => Some(ended_turn(connection, &id, end, walk.answer)?),
        None => None,
    };
    Ok(SummaryFacts {
        phase: state.phase,
        revision: decode("nodes", "output_revision", u64::try_from(revision))?,
        last: walk.last,
        last_turn,
        last_attachment,
    })
}

/// Walks the root's block rows back from its last one, until it knows the
/// latest terminal block and the latest ended turn with its last text. A
/// turn's input blocks carry its id, and the turn lasts until a block of
/// another turn's input, so a text belongs to the ended turn when it comes
/// after one of that turn's input blocks. While the root is in a turn
/// (`phase` is not idle) that turn has not ended, so what the walk read of
/// it, such as the responses of its earlier requests, counts for nothing.
/// An action that has written no block of its own yet, such as a `compact`
/// or a send before its message is written, cannot be told from the turn
/// before it, which is then passed over too until the action writes one.
fn walk(
    rows: impl Iterator<Item = Result<WalkRow, StorageError>>,
    phase: SessionPhase,
) -> Result<Walk, StorageError> {
    let mut last = None;
    // The running turn, while the walk is still inside it.
    let mut running: Option<Option<String>> = (phase != SessionPhase::Idle).then_some(None);
    let mut search = EndSearch::default();
    let mut found = false;
    for row in rows {
        let row = row?;
        if last.is_none() {
            last = match row.kind.as_str() {
                "response" => Some(Terminal::Response),
                "error" => Some(Terminal::Error),
                "abort" => Some(Terminal::Abort),
                _ => None,
            };
        }
        if let Some(turn) = &running
            && let Some(input) = &row.turn
        {
            if turn.as_ref().is_none_or(|turn| turn == input) {
                running = Some(Some(input.clone()));
                search = EndSearch::default();
                found = false;
                continue;
            }
            running = None;
        }
        if !found {
            found = search.step(row);
        }
        if found && running.is_none() {
            break;
        }
    }
    Ok(Walk {
        last,
        end: search.end,
        answer: search.answer,
    })
}

/// The turn the terminal block at `end` ended, whose last text is `answer`.
fn ended_turn(
    connection: &Connection,
    node: &str,
    end: i64,
    answer: Option<String>,
) -> Result<EndedTurn, StorageError> {
    let block: String = connection.query_row(
        "SELECT block FROM blocks WHERE node_id = ?1 AND idx = ?2",
        params![node, end],
        |row| row.get(0),
    )?;
    let (id, outcome) = match json::<Block>("blocks", "block", &block)? {
        Block::Response(block) => (block.id, Terminal::Response),
        Block::Error(block) => (block.id, Terminal::Error),
        Block::Abort(block) => (block.id, Terminal::Abort),
        _ => unreachable!("the walk ends a turn only at a terminal block"),
    };
    Ok(EndedTurn {
        id,
        outcome,
        answer,
    })
}

/// Which saved state of the root's transcript a reader saw (`storage.md`
/// § Search index): its output revision and its block count. Every save
/// that changes the transcript changes one of them: a save of output, or one
/// that removes rows, advances the revision, and a save of input alone adds
/// blocks. The transcript's own version, its epoch and revision
/// (`runtime.md` § Patches and versions), lives only in the running session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SavedVersion {
    pub revision: u64,
    pub blocks: u64,
}

/// The saved version of the root's transcript; the empty one, revision 0
/// with no block, when the tree has no root yet.
pub fn root_version(connection: &Connection) -> Result<SavedVersion, StorageError> {
    Ok(saved_root(connection)?.map_or_else(SavedVersion::default, |(_, version)| version))
}

/// The root's blocks with the saved version they make, read together; none
/// and the empty version when the tree has no root yet.
pub fn root_transcript(connection: &Connection) -> Result<(SavedVersion, Vec<Block>), StorageError> {
    let Some((root, version)) = saved_root(connection)? else {
        return Ok((SavedVersion::default(), Vec::new()));
    };
    let blocks = blocks_of(connection, &root, integer(version.blocks))?;
    Ok((version, blocks))
}

/// The root's id and the saved version of its transcript.
fn saved_root(connection: &Connection) -> Result<Option<(NodeId, SavedVersion)>, StorageError> {
    let root: Option<(String, i64, i64)> = connection
        .query_row(
            "SELECT id, output_revision, block_count FROM nodes WHERE parent_id IS NULL",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((root, revision, blocks)) = root else {
        return Ok(None);
    };
    let version = SavedVersion {
        revision: decode("nodes", "output_revision", u64::try_from(revision))?,
        blocks: decode("nodes", "block_count", u64::try_from(blocks))?,
    };
    Ok(Some((decode("nodes", "id", NodeId::try_from(root))?, version)))
}

/// Whether the tree `connection` holds has its root; a Fork's destination
/// is committed once it does.
pub fn has_root(connection: &Connection) -> Result<bool, StorageError> {
    let exists = connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM nodes WHERE parent_id IS NULL)",
        [],
        |row| row.get(0),
    )?;
    Ok(exists)
}

/// The root's blocks as the database holds them; none when the tree has
/// no root yet.
pub fn root_blocks(connection: &Connection) -> Result<Vec<Block>, StorageError> {
    let root: Option<(String, i64)> = connection
        .query_row(
            "SELECT id, block_count FROM nodes WHERE parent_id IS NULL",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((root, block_count)) = root else {
        return Ok(Vec::new());
    };
    blocks_of(connection, &decode("nodes", "id", NodeId::try_from(root))?, block_count)
}

/// A transcript a page is read from (`web-api.md` § Pages): an agent's in
/// the tree, or what a share's list shows of one (`sharing.md` § Storage and
/// routes). The pages, their light form and a block read whole are the same
/// for both; only the rows they come from differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transcript {
    /// The transcript of the node, the blocks below its block count.
    Agent(NodeId),
    /// The list of the share's token for the agent of that number, 0 for the
    /// root's: each block at its position on the list, read from the row
    /// the entry names or from the share's copy. The list is the whole of
    /// what it reads: no read reaches a row the list does not name.
    Shared { token: ShareToken, agent: u64 },
}

/// The blocks of a share's list, as the `FROM` of a query that names them
/// `entry` and `block`: each entry with the block it shows, its row's or
/// its copy's. `?1` is the token and `?2` the agent.
const LISTED: &str = "share_entries AS entry
  LEFT JOIN blocks AS row ON row.node_id = entry.node_id AND row.idx = entry.idx
  LEFT JOIN share_blocks AS copy
    ON entry.node_id IS NULL AND copy.token = entry.token AND copy.agent = entry.agent AND copy.idx = entry.position";

impl Transcript {
    /// The token and the agent of a share's list, as `?1` and `?2`.
    fn listed(token: &ShareToken, agent: u64) -> [Value; 2] {
        // A number past the column's names no agent, as a query may: -1
        // matches no entry.
        [
            Value::Text(token.as_str().to_owned()),
            Value::Integer(i64::try_from(agent).unwrap_or(-1)),
        ]
    }

    /// How many blocks the transcript holds; none when there is no such
    /// transcript: a node the tree lacks, or an agent the list holds no
    /// block of, but the root's, which a share always holds.
    pub fn length(&self, connection: &Connection) -> Result<Option<usize>, StorageError> {
        let length: Option<i64> = match self {
            Self::Agent(node) => connection
                .query_row(
                    "SELECT block_count FROM nodes WHERE id = ?1",
                    [node.as_str()],
                    |row| row.get(0),
                )
                .optional()?,
            Self::Shared { token, agent } => {
                let held: i64 = connection.query_row(
                    "SELECT COUNT(*) FROM share_entries WHERE token = ?1 AND agent = ?2",
                    params_from_iter(Self::listed(token, *agent)),
                    |row| row.get(0),
                )?;
                (held > 0 || *agent == 0).then_some(held)
            }
        };
        length
            .map(|length| decode("nodes", "block_count", usize::try_from(length)))
            .transpose()
    }

    /// The indices of the `user` blocks among its first `length` blocks,
    /// where its requests start. A share's are found through the indices of
    /// block types, the transcript's and the copies', without reading a
    /// block.
    pub fn user_blocks(&self, connection: &Connection, length: usize) -> Result<Vec<usize>, StorageError> {
        let (sql, parameters): (&str, Vec<Value>) = match self {
            Self::Agent(node) => (
                "SELECT idx FROM blocks WHERE node_id = ?1 AND json_extract(block, '$.type') = 'user' AND idx < ?2
                 ORDER BY idx",
                vec![Value::Text(node.as_str().to_owned()), Value::Integer(count(length))],
            ),
            Self::Shared { token, agent } => (
                // From the agent's user blocks in the index of block types
                // to their entries, so no other block is visited.
                "SELECT entry.position FROM nodes
                   CROSS JOIN blocks AS row INDEXED BY blocks_by_type
                     ON row.node_id = nodes.id AND json_extract(row.block, '$.type') = 'user'
                   CROSS JOIN share_entries AS entry INDEXED BY share_entries_by_row
                     ON entry.token = ?1 AND entry.node_id = row.node_id AND entry.idx = row.idx
                 WHERE nodes.number = ?2 AND entry.agent = ?2 AND entry.position < ?3
                 UNION
                 SELECT entry.position FROM share_blocks AS copy INDEXED BY share_blocks_by_type
                   CROSS JOIN share_entries AS entry
                     ON entry.token = copy.token AND entry.agent = copy.agent AND entry.position = copy.idx
                 WHERE copy.token = ?1 AND copy.agent = ?2 AND json_extract(copy.block, '$.type') = 'user'
                   AND entry.node_id IS NULL AND entry.position < ?3
                 ORDER BY 1",
                [Self::listed(token, *agent).as_slice(), &[Value::Integer(count(length))]].concat(),
            ),
        };
        let mut statement = connection.prepare(sql)?;
        let mut rows = statement.query(params_from_iter(parameters))?;
        let mut users = Vec::new();
        while let Some(row) = rows.next()? {
            let index: i64 = row.get(0)?;
            users.push(decode(self.table(), "idx", usize::try_from(index))?);
        }
        Ok(users)
    }

    /// Its blocks at the indices `range`, all of which it holds.
    pub fn blocks_in(&self, connection: &Connection, range: Range<usize>) -> Result<Vec<Block>, StorageError> {
        let bounds = [Value::Integer(count(range.start)), Value::Integer(count(range.end))];
        let (sql, parameters): (String, Vec<Value>) = match self {
            Self::Agent(node) => (
                "SELECT idx, block FROM blocks WHERE node_id = ?1 AND idx >= ?2 AND idx < ?3 ORDER BY idx".to_owned(),
                [&[Value::Text(node.as_str().to_owned())][..], &bounds].concat(),
            ),
            Self::Shared { token, agent } => (
                format!(
                    "SELECT entry.position, coalesce(row.block, copy.block) FROM {LISTED}
                     WHERE entry.token = ?1 AND entry.agent = ?2 AND entry.position >= ?3 AND entry.position < ?4
                     ORDER BY entry.position"
                ),
                [Self::listed(token, *agent).as_slice(), &bounds].concat(),
            ),
        };
        let mut statement = connection.prepare(&sql)?;
        let mut rows = statement.query(params_from_iter(parameters))?;
        let mut blocks = Vec::with_capacity(range.len());
        while let Some(row) = rows.next()? {
            let index: i64 = row.get(0)?;
            // A share's entry whose row or copy is gone holds no block.
            let text: Option<String> = row.get(1)?;
            let (true, Some(text)) = (index == count(range.start + blocks.len()), text) else {
                return Err(self.gap(range.start + blocks.len()));
            };
            blocks.push(json(self.table(), "block", &text)?);
        }
        if blocks.len() != range.len() {
            return Err(self.gap(range.start + blocks.len()));
        }
        Ok(blocks)
    }

    /// The index of the block `id` among its first `length` blocks. A
    /// share's is the position of the entry that names the very row the id
    /// is found in, or the copy: a block of the same id elsewhere is not one
    /// it holds.
    pub fn block_index(
        &self,
        connection: &Connection,
        id: &BlockId,
        length: usize,
    ) -> Result<Option<usize>, StorageError> {
        let wanted = [Value::Text(id.as_str().to_owned()), Value::Integer(count(length))];
        let (sql, parameters): (&str, Vec<Value>) = match self {
            Self::Agent(node) => (
                "SELECT idx FROM blocks WHERE node_id = ?1 AND json_extract(block, '$.id') = ?2 AND idx < ?3",
                [&[Value::Text(node.as_str().to_owned())][..], &wanted].concat(),
            ),
            Self::Shared { token, agent } => (
                // From the rows of that id to the entry that names one.
                "SELECT entry.position FROM blocks AS row INDEXED BY blocks_by_id
                   CROSS JOIN share_entries AS entry INDEXED BY share_entries_by_row
                     ON entry.token = ?1 AND entry.node_id = row.node_id AND entry.idx = row.idx
                 WHERE json_extract(row.block, '$.id') = ?3 AND entry.agent = ?2 AND entry.position < ?4
                 UNION ALL
                 SELECT entry.position FROM share_blocks AS copy
                   JOIN share_entries AS entry
                     ON entry.token = copy.token AND entry.agent = copy.agent AND entry.position = copy.idx
                 WHERE copy.token = ?1 AND copy.agent = ?2 AND json_extract(copy.block, '$.id') = ?3
                   AND entry.node_id IS NULL AND entry.position < ?4
                 LIMIT 1",
                [Self::listed(token, *agent).as_slice(), &wanted].concat(),
            ),
        };
        let index: Option<i64> = connection
            .query_row(sql, params_from_iter(parameters), |row| row.get(0))
            .optional()?;
        index
            .map(|index| decode(self.table(), "idx", usize::try_from(index)))
            .transpose()
    }

    /// The entries of the newest instructions block among its first
    /// `length` blocks, which the context card lists; none before the first
    /// (`instructions.md` § What the card lists), and none in a share,
    /// which shows no `context` block.
    pub fn newest_instructions(
        &self,
        connection: &Connection,
        length: usize,
    ) -> Result<Vec<InstructionEntry>, StorageError> {
        let Self::Agent(node) = self else {
            return Ok(Vec::new());
        };
        let text: Option<String> = connection
            .query_row(
                "SELECT block FROM blocks
                 WHERE node_id = ?1 AND json_extract(block, '$.type') = 'context' AND idx < ?2
                   AND json_extract(block, '$.source') = ?3
                 ORDER BY idx DESC LIMIT 1",
                params![node.as_str(), count(length), INSTRUCTIONS_SOURCE],
                |row| row.get(0),
            )
            .optional()?;
        let Some(text) = text else {
            return Ok(Vec::new());
        };
        match json("blocks", "block", &text)? {
            Block::Context(context) => Ok(context.instructions),
            _ => unreachable!("the query selects a context block"),
        }
    }

    fn table(&self) -> &'static str {
        match self {
            Self::Agent(_) => "blocks",
            Self::Shared { .. } => "share_entries",
        }
    }

    fn gap(&self, index: usize) -> StorageError {
        match self {
            Self::Agent(node) => gap(node, index),
            Self::Shared { token, agent } => StorageError::Corrupt {
                table: "share_entries",
                column: "position",
                reason: format!("agent {agent} of share {token} has no block {index}"),
            },
        }
    }
}

/// What a page read found (`web-api.md` § Pages).
pub enum PageRead {
    /// The page: its first block's index, the transcript's length, its
    /// blocks whole, the entries of the newest instructions block, and the
    /// summary size of each compaction marker's boundary.
    Page {
        start: usize,
        length: usize,
        blocks: Vec<Block>,
        instructions: Vec<InstructionEntry>,
        summaries: Vec<CompactionSummary>,
    },
    NoNode,
    NoBlock,
    /// `before` or `after` an index the transcript does not reach.
    Outside,
    Changed,
    Invalid(&'static str),
}

/// Where a page is read from, as its query asks (`web-api.md` § Pages):
/// none, `before` or `after` with its `edge`, or `around`.
#[derive(Debug, Clone, Copy)]
pub struct PageQuery<'a> {
    pub before: Option<u32>,
    pub after: Option<u32>,
    pub around: Option<&'a BlockId>,
    pub edge: Option<&'a BlockId>,
}

impl<'a> From<&'a TranscriptQuery> for PageQuery<'a> {
    fn from(query: &'a TranscriptQuery) -> Self {
        Self {
            before: query.before,
            after: query.after,
            around: query.around.as_ref(),
            edge: query.edge.as_ref(),
        }
    }
}

/// The page `query` asks of `transcript`, its blocks whole: whole requests
/// within the page budget (`web-api.md` § Pages).
pub fn read_page(
    connection: &Connection,
    transcript: &Transcript,
    query: PageQuery<'_>,
) -> Result<PageRead, StorageError> {
    let Some(length) = transcript.length(connection)? else {
        return Ok(PageRead::NoNode);
    };
    // The index a page extends from, while the block there is still the
    // `edge` the page holds; otherwise why the read is refused.
    let edge_at = |index: u32| -> Result<Result<usize, PageRead>, StorageError> {
        let Some(edge) = query.edge else {
            return Ok(Err(PageRead::Invalid("`before` and `after` take an `edge`")));
        };
        let index = usize::try_from(index).unwrap_or(usize::MAX);
        if index >= length {
            return Ok(Err(PageRead::Outside));
        }
        let held = transcript.blocks_in(connection, index..index + 1)?;
        Ok(if held[0].id() == edge { Ok(index) } else { Err(PageRead::Changed) })
    };
    let at = match (query.before, query.after, query.around) {
        (None, None, None) => PageAt::Latest,
        (Some(index), None, None) => match edge_at(index)? {
            Ok(index) => PageAt::Before(index),
            Err(refusal) => return Ok(refusal),
        },
        (None, Some(index), None) => match edge_at(index)? {
            Ok(index) => PageAt::After(index),
            Err(refusal) => return Ok(refusal),
        },
        (None, None, Some(block)) => match transcript.block_index(connection, block, length)? {
            Some(index) => PageAt::Around(index),
            None => return Ok(PageRead::NoBlock),
        },
        _ => return Ok(PageRead::Invalid("one of `before`, `after` and `around`")),
    };
    let users = transcript.user_blocks(connection, length)?;
    let requests = requests(length, &users);
    // Each request the selection sizes is read once, and the page is made
    // of those it keeps.
    let mut read: HashMap<usize, Vec<Block>> = HashMap::new();
    let mut failed = None;
    let range = page(&requests, at, |request| {
        match transcript.blocks_in(connection, request.clone()) {
            Ok(blocks) => {
                let bytes = light_bytes(&blocks);
                read.insert(request.start, blocks);
                bytes
            }
            Err(error) => {
                failed.get_or_insert(error);
                0
            }
        }
    });
    if let Some(error) = failed {
        return Err(error);
    }
    let blocks: Vec<Block> = requests
        .iter()
        .filter(|request| range.start <= request.start && request.end <= range.end)
        .flat_map(|request| {
            read.remove(&request.start)
                .expect("each request of the page was read to size it")
        })
        .collect();
    let mut summaries = Vec::new();
    for block in &blocks {
        if let Block::CompactionMarker(marker) = block
            && let Some(index) = transcript.block_index(connection, &marker.boundary_id, length)?
            && let Block::CompactionBoundary(boundary) =
                &transcript.blocks_in(connection, index..index + 1)?[0]
        {
            summaries.push(CompactionSummary {
                marker: marker.id.clone(),
                summary_tokens: boundary.summary_tokens,
            });
        }
    }
    Ok(PageRead::Page {
        start: range.start,
        length,
        blocks,
        instructions: transcript.newest_instructions(connection, length)?,
        summaries,
    })
}

/// What a read of one block whole found (`web-api.md` § Light form).
pub enum BlockRead {
    Block(Block),
    NoNode,
    NoBlock,
}

/// The block `id` of `transcript`, whole.
pub fn read_block(
    connection: &Connection,
    transcript: &Transcript,
    id: &BlockId,
) -> Result<BlockRead, StorageError> {
    let Some(length) = transcript.length(connection)? else {
        return Ok(BlockRead::NoNode);
    };
    let Some(index) = transcript.block_index(connection, id, length)? else {
        return Ok(BlockRead::NoBlock);
    };
    let mut blocks = transcript.blocks_in(connection, index..index + 1)?;
    Ok(BlockRead::Block(blocks.remove(0)))
}

/// The subagents of the tree, in the order they started, each with the
/// model selection its checkpoint saved.
pub fn subagents(
    connection: &Connection,
) -> Result<Vec<(NodeRecord, ModelSelection)>, StorageError> {
    let mut statement = connection.prepare(&format!(
        "SELECT {NODE_COLUMNS}, json_extract(state, '$.model') AS model FROM nodes
         WHERE parent_id IS NOT NULL ORDER BY number"
    ))?;
    let mut rows = statement.query([])?;
    let mut nodes = Vec::new();
    while let Some(row) = rows.next()? {
        let model: String = row.get("model")?;
        nodes.push((node_row(row)?, json("nodes", "state", &model)?));
    }
    Ok(nodes)
}

/// The call that started the command `command`, with the node that ran
/// it: of the calls whose view names the command, the first, since a stored
/// look at a command, from before such looks were removed, came after it;
/// of calls equally early, the oldest agent's, since a fork's copy of its
/// source's call keeps the call's time (`subagents.md` § Fork).
pub fn command_call(
    connection: &Connection,
    command: &CommandId,
) -> Result<Option<(NodeId, ToolCallBlock)>, StorageError> {
    let row: Option<(String, String)> = connection
        .query_row(
            "SELECT blocks.node_id, blocks.block FROM blocks JOIN nodes ON nodes.id = blocks.node_id
             WHERE json_extract(blocks.block, '$.view.commandId') = ?1
               AND blocks.idx < nodes.block_count
             ORDER BY json_extract(blocks.block, '$.createdAt'), nodes.number LIMIT 1",
            [command.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((node, text)) = row else {
        return Ok(None);
    };
    match json("blocks", "block", &text)? {
        Block::ToolCall(call) => Ok(Some((decode("blocks", "node_id", NodeId::try_from(node))?, call))),
        _ => Err(StorageError::Corrupt {
            table: "blocks",
            column: "block",
            reason: format!("a block that is not a call names command {command}"),
        }),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::BTreeMap;
    use std::num::NonZeroUsize;
    use std::sync::Mutex;

    use demi_agent_store::testing::{store_contract, test_model, text};
    use demi_shared_types::{
        B64Bytes, BlobRef, ResponseBlock, TextBlock, TokenUsage, TurnId, UserBlock,
    };
    use demi_web_api_protocol::ids::ConversationId;

    use super::*;
    use crate::conversations::ConversationStores;

    /// The owner's blobs in memory, named by their SHA-256 as the object
    /// store names them.
    #[derive(Default)]
    struct Blobs(Mutex<BTreeMap<BlobRef, B64Bytes>>);

    impl BlobStore for Blobs {
        fn put(&self, bytes: B64Bytes) -> LocalBoxFuture<'_, Result<BlobRef, StoreError>> {
            let blob = BlobRef::of(bytes.as_bytes());
            self.0.lock().unwrap().insert(blob.clone(), bytes);
            Box::pin(async move { Ok(blob) })
        }

        fn get<'a>(
            &'a self,
            blob: &'a BlobRef,
        ) -> LocalBoxFuture<'a, Result<Option<B64Bytes>, StoreError>> {
            let bytes = self.0.lock().unwrap().get(blob).cloned();
            Box::pin(async move { Ok(bytes) })
        }
    }

    /// A tree store over a new conversation database and its owner's blob
    /// namespace, with the stores that hold it.
    pub(crate) async fn store() -> (SqliteTreeStore, ConversationStores, tempfile::TempDir) {
        let data = tempfile::tempdir().unwrap();
        let stores = ConversationStores::open(
            data.path().join("conversations"),
            NonZeroUsize::new(4).unwrap(),
            std::sync::Arc::new(crate::schema::NoBlobs),
        )
        .await
        .unwrap();
        let store = SqliteTreeStore::new(
            stores.db(&conversation()),
            Arc::new(Blobs::default()),
            Rc::new(|_: &NodeId| {}),
        );
        (store, stores, data)
    }

    pub(crate) fn conversation() -> ConversationId {
        ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b").unwrap()
    }

    pub(crate) fn id(value: &str) -> NodeId {
        NodeId::try_from(value).unwrap()
    }

    /// A node in its first round, agent `number` of the conversation.
    pub(crate) fn record(node: &str, parent: Option<&str>, number: u64) -> NodeRecord {
        NodeRecord {
            id: id(node),
            number,
            parent: parent.map(id),
            description: format!("{node} task"),
            profile: None,
            instructions: None,
            context_limit: None,
            round: 1,
            started_at: Timestamp::UNIX_EPOCH,
            can_spawn_subagents: true,
            closed: None,
            delivered: false,
            forked_from: None,
            cwd: parent.map(|_| ChildDirectory {
                cwd: "/w".to_owned(),
                under: Placement {
                    device: Some("laptop".to_owned()),
                    directory: "/w".to_owned(),
                },
            }),
        }
    }

    pub(crate) fn state() -> CheckpointState {
        CheckpointState {
            phase: SessionPhase::Idle,
            queue: Vec::new(),
            agent_inputs: Vec::new(),
            reports: Vec::new(),
            intervals: Vec::new(),
            model: test_model(),
            edits: Vec::new(),
            account: None,
        }
    }

    pub(crate) fn update(blocks: Vec<(usize, Block)>, block_count: usize) -> CheckpointUpdate {
        CheckpointUpdate {
            state: state(),
            changed_blocks: blocks,
            block_count,
        }
    }

    pub(crate) fn user(block: &str) -> Block {
        Block::User(UserBlock {
            id: block.try_into().unwrap(),
            turn_id: TurnId::try_from("t1").unwrap(),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            content: text("hi"),
            preamble: None,
            entries: Vec::new(),
        })
    }

    pub(crate) fn reply(block: &str) -> Block {
        Block::Text(TextBlock {
            id: block.try_into().unwrap(),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            text: "hello".into(),
            forkable: false,
            entries: Vec::new(),
        })
    }

    pub(crate) fn response(block: &str) -> Block {
        Block::Response(ResponseBlock {
            id: block.try_into().unwrap(),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            usage: Some(TokenUsage::default()),
        })
    }

    /// A completed `shell` call that started the command `command`.
    pub(crate) fn shell_call(block: &str, command: &str) -> Block {
        serde_json::from_value(serde_json::json!({
            "type": "tool_call",
            "id": block,
            "createdAt": Timestamp::UNIX_EPOCH,
            "model": test_model(),
            "toolUseId": "call-1",
            "toolName": "shell",
            "input": "{}",
            "status": "completed",
            "output": [],
            "endedAt": Timestamp::UNIX_EPOCH,
            "view": {
                "kind": "shell",
                "status": "exited",
                "commandId": command,
                "exitCode": 0,
                "runningMs": 1,
                "idleMs": 0,
                "chunks": [],
                "viewTruncated": false,
            },
        }))
        .unwrap()
    }

    async fn facts(stores: &ConversationStores) -> SummaryFacts {
        stores
            .read(&conversation(), summary)
            .await
            .unwrap()
            .unwrap()
    }

    // A fork of a child copies the child's calls, with their times
    // (`subagents.md` § Fork); the call that started a command is still the
    // source's, also when the fork saved its copy before the source saved
    // the call, as a fork of a running child can.
    #[tokio::test(flavor = "local")]
    async fn a_commands_call_is_its_source_agents_when_a_fork_holds_a_copy() {
        let (tree, stores, _data) = store().await;
        tree.create_node(record("root", None, 0), update(Vec::new(), 0))
            .await
            .unwrap();
        tree.create_node(record("source", Some("root"), 1), update(Vec::new(), 0))
            .await
            .unwrap();
        let call = || vec![(0, user("u1")), (1, shell_call("call", "c1"))];
        tree.create_node(record("fork", Some("root"), 2), update(call(), 2))
            .await
            .unwrap();
        tree.session_store(&id("source"))
            .save(update(call(), 2))
            .await
            .unwrap();

        let command = CommandId::try_from("c1").unwrap();
        let found = stores
            .read(&conversation(), move |connection| command_call(connection, &command))
            .await
            .unwrap()
            .unwrap()
            .map(|(node, _)| node);
        assert_eq!(found, Some(id("source")));
    }

    #[tokio::test(flavor = "local")]
    async fn output_advances_the_revision_and_input_alone_does_not() {
        let (tree, stores, _data) = store().await;
        assert_eq!(stores.read(&conversation(), summary).await.unwrap(), None);
        tree.create_node(record("root", None, 0), update(Vec::new(), 0))
            .await
            .unwrap();
        let root = tree.session_store(&id("root"));
        assert_eq!(facts(&stores).await, SummaryFacts::EMPTY);

        root.save(update(vec![(0, user("u1"))], 1)).await.unwrap();
        assert_eq!(facts(&stores).await.revision, 0, "the user's message alone");
        root.save(update(vec![(1, reply("a1")), (2, response("r1"))], 3))
            .await
            .unwrap();
        let answered = facts(&stores).await;
        assert_eq!(
            (answered.revision, answered.last),
            (1, Some(Terminal::Response))
        );
        // A rewrite that drops rows is a change of the output.
        root.save(update(Vec::new(), 1)).await.unwrap();
        let rewritten = facts(&stores).await;
        assert_eq!((rewritten.revision, rewritten.last), (2, None));
    }

    #[tokio::test(flavor = "local")]
    async fn the_database_keeps_the_tree_store_contract() {
        let (tree, _stores, _data) = store().await;
        store_contract::create_queues_the_first_message_with_the_node_and_a_save_replaces_it(&tree)
            .await;
        let (tree, _stores, _data) = store().await;
        store_contract::a_save_delivers_the_completions_its_transcript_carries_and_only_those(
            &tree,
        )
        .await;
        let (tree, _stores, _data) = store().await;
        store_contract::reopen_makes_a_closed_node_live_with_its_message_and_delete_takes_the_subtree(&tree).await;
        let (tree, _stores, _data) = store().await;
        store_contract::a_completion_of_an_earlier_round_marks_the_current_one_undelivered(&tree)
            .await;
        let (tree, _stores, _data) = store().await;
        store_contract::a_save_delivers_a_completion_it_holds_as_waiting_input(&tree).await;
        let (tree, _stores, _data) = store().await;
        store_contract::children_list_in_spawn_order_and_a_close_keeps_its_result(&tree).await;
        let (tree, _stores, _data) = store().await;
        store_contract::the_blob_namespace_names_bytes_by_their_sha256(&tree).await;
        let (tree, _stores, _data) = store().await;
        store_contract::each_sequence_gives_its_numbers_once_in_order(&tree).await;
    }
}
