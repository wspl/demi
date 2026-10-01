//! The tree store over a conversation's database (`runtime.md` § Tree store,
//! `storage.md` § Conversation state and transactions): a node's record is
//! its `nodes` row, and its checkpoint is that row's state with the node's
//! `blocks` rows and its command state (`command_snapshots`,
//! `session_boundaries`). Creating, saving, closing, reopening and deleting
//! a node are each one transaction on the conversation's writer connection,
//! where rows are also serialized and decoded. Every row read is decoded and
//! checked, and corrupt data stops the read.
//!
//! Blocks hold their media by reference, and a checkpoint that holds media
//! bytes is refused; each node's session reaches the conversation owner's
//! blob namespace through its store (`storage.md` § Attachment and
//! transcript media). Every block written or removed changes the
//! `blob_refs` index in the same transaction ([`blob_refs`](super::blob_refs)),
//! and the commit records the uses of the blobs it names before it commits.
//!
//! The same readings serve what the web app reads without a live session: a
//! conversation's summary facts and its history, on a read-only connection,
//! which keep the media references.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use demi_agent_store::{AgentTreeStore, SessionStore, StoredOutput};
use demi_agent_store::{
    BoundaryEdge, Checkpoint, CheckpointState, CheckpointUpdate, ClosePhase, CommandStateSnapshot,
    CommandStorageKey, CommandVersion, CommitGuard, NodeClose, NodeRecord, SessionBoundary,
    StoreError, media::BlobStore,
};
use demi_backend_remote_host::decode_output;
use demi_shared_types::{
    Block, BlockId, CommandId, CompletionId, NodeId, QueuedMessage, Sequence, SessionPhase,
    Timestamp,
};
use futures_util::future::LocalBoxFuture;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use serde_json::Value;

use super::StorageError;
use super::blob_refs::{self, OwnerBlobs};
use super::columns::{count, decode, instant, json, to_json};
use super::command_outputs::{self, OutputRow};
use super::conversations::ConversationDb;
use super::sequences;

/// What a tree store tells after each commit of a node's checkpoint, with
/// the node: a conversation's summary reads its root's.
pub type Saved = Rc<dyn Fn(&NodeId)>;

/// One conversation's agent tree in its database, with its owner's blobs.
pub struct SqliteTreeStore {
    db: ConversationDb,
    blobs: Arc<dyn OwnerBlobs>,
    saved: Saved,
}

impl SqliteTreeStore {
    pub fn new(db: ConversationDb, blobs: Arc<dyn OwnerBlobs>, saved: Saved) -> Self {
        Self { db, blobs, saved }
    }
}

/// One node's checkpoint in its conversation's database.
struct SqliteSessionStore {
    db: ConversationDb,
    blobs: Arc<dyn OwnerBlobs>,
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

    fn create_node(
        &self,
        record: NodeRecord,
        initial: CheckpointUpdate,
    ) -> LocalBoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            let completions = initial.carried_completions()?;
            let node = record.id.clone();
            let blobs = self.blobs.clone();
            self.db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    if node_by_id(&transaction, &record.id)?.is_some() {
                        return Ok(Err(StoreError::Failed(format!("node {} already exists", record.id))));
                    }
                    let (phase, closed_at, result, failure) = close_columns(record.closed.as_ref());
                    transaction.execute(
                        "INSERT INTO nodes (id, number, parent_id, description, profile, round, started_at, can_spawn,
                           closed_phase, closed_at, result, failure, delivered, state, block_count, command_revision,
                           output_revision)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 0, 0, 0)",
                        params![
                            record.id.as_str(),
                            integer(record.number),
                            record.parent.as_ref().map(NodeId::as_str),
                            record.description,
                            record.profile,
                            integer(record.round),
                            record.started_at.as_millisecond(),
                            record.can_spawn_subagents,
                            phase,
                            closed_at,
                            result,
                            failure,
                            record.delivered,
                            to_json(&initial.state),
                        ],
                    )?;
                    let command_state = initial.command_state.clone().unwrap_or_else(CommandStateSnapshot::initial);
                    let initial = CheckpointUpdate {
                        command_state: Some(command_state),
                        ..initial
                    };
                    let written = write_checkpoint(&transaction, &*blobs, &record.id, &initial, &completions)?;
                    if written.is_ok() {
                        transaction.commit()?;
                    }
                    Ok(written)
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
        let blobs = self.blobs.clone();
        Box::pin(async move {
            // The node's descendants and every row of theirs, their index
            // rows included, go with it through the cascades.
            self.db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    let removed = blob_refs::subtree(&transaction, &node)?;
                    if let Err(refused) = blobs.commit_uses(&removed) {
                        return Ok(Err(refused));
                    }
                    transaction.execute("DELETE FROM nodes WHERE id = ?1", [node.as_str()])?;
                    transaction.commit()?;
                    Ok(Ok(()))
                })
                .await
                .map_err(store_error)?
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

    /// Reads the command's row on a read-only connection, then its blob,
    /// which it decodes on the blocking pool: a stored output takes up to
    /// 16 MiB.
    fn command_output<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<Option<StoredOutput>, StoreError>> {
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
                OutputRow::NotStored(reason) => return Ok(Some(StoredOutput::NotStored(reason))),
                OutputRow::Removed(at) => return Ok(Some(StoredOutput::Removed(at))),
            };
            let bytes = self.blobs.media().get(&blob).await?.ok_or_else(|| {
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
            Ok(Some(StoredOutput::Stored(output)))
        })
    }
}

impl SessionStore for SqliteSessionStore {
    fn save<'a>(
        &'a self,
        update: CheckpointUpdate,
        guard: &'a CommitGuard,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        let guard = guard.clone();
        let node = self.node.clone();
        Box::pin(async move {
            let completions = update.carried_completions()?;
            let commit = self.db.commit_point();
            let blobs = self.blobs.clone();
            self.db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    // The guard is checked at the start of the transaction, so
                    // an invocation a rewrite made stale commits nothing.
                    if let Err(stale) = guard.check() {
                        return Ok(Err(stale));
                    }
                    let written =
                        write_checkpoint(&transaction, &*blobs, &node, &update, &completions)?;
                    if written.is_ok() {
                        commit.commit(transaction)?;
                    }
                    Ok(written)
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
        self.blobs.media()
    }
}

/// Writes one save of `node` in `transaction`: the changed block rows, the
/// rows past the new end gone, each with its index rows, the state row, the
/// command state when it changed, and the child completions it carries
/// marked delivered; last, the uses of the blobs whose references it wrote
/// or removed. Changed output advances the node's output revision; input
/// alone does not. A refusal leaves the transaction uncommitted.
fn write_checkpoint(
    transaction: &Transaction<'_>,
    blobs: &dyn OwnerBlobs,
    node: &NodeId,
    update: &CheckpointUpdate,
    completions: &[CompletionId],
) -> Result<Result<(), StoreError>, StorageError> {
    let mut touched = Vec::new();
    for (index, block) in &update.changed_blocks {
        blob_refs::write_block(transaction, node, *index, block, &mut touched)?;
    }
    blob_refs::truncate(transaction, node, update.block_count, &mut touched)?;
    let block_count = count(update.block_count);
    let output = update
        .changed_blocks
        .iter()
        .any(|(_, block)| is_output(block));
    let revision = update
        .command_state
        .as_ref()
        .map(|state| integer(state.revision));
    // The old block count is the column's value before the update: rows
    // gone are a rewrite of the output.
    let changed = transaction.execute(
        "UPDATE nodes SET state = ?2, block_count = ?3, command_revision = COALESCE(?5, command_revision),
           output_revision = output_revision + (CASE WHEN block_count > ?3 OR ?4 THEN 1 ELSE 0 END)
         WHERE id = ?1",
        params![node.as_str(), to_json(&update.state), block_count, output, revision],
    )?;
    if changed == 0 {
        return Ok(Err(missing(node)));
    }
    if let Some(command_state) = &update.command_state
        && let Err(refusal) = write_command_state(transaction, node, command_state)?
    {
        return Ok(Err(refusal));
    }
    for round in completions {
        transaction.execute(
            "UPDATE nodes SET delivered = 1 WHERE id = ?1 AND parent_id = ?2 AND round = ?3",
            params![round.child.as_str(), node.as_str(), integer(round.round)],
        )?;
    }
    Ok(blobs.commit_uses(&touched))
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

/// Writes a node's command state whole: its versions, which are immutable,
/// and its boundaries. A version the store holds with other values is
/// refused, compared as values, not as text.
fn write_command_state(
    transaction: &Transaction<'_>,
    node: &NodeId,
    state: &CommandStateSnapshot,
) -> Result<Result<(), StoreError>, StorageError> {
    for version in &state.versions {
        let stored: Option<String> = transaction
            .query_row(
                "SELECT entries FROM command_snapshots WHERE node_id = ?1 AND revision = ?2",
                params![node.as_str(), integer(version.revision)],
                |row| row.get(0),
            )
            .optional()?;
        match stored {
            Some(entries) => {
                if version_values(&entries)? != version.values {
                    return Ok(Err(StoreError::Failed(format!(
                        "command-state version {} is immutable",
                        version.revision
                    ))));
                }
            }
            None => {
                transaction.execute(
                    "INSERT INTO command_snapshots (node_id, revision, entries) VALUES (?1, ?2, ?3)",
                    params![node.as_str(), integer(version.revision), to_json(&version.values)],
                )?;
            }
        }
    }
    let revisions: Vec<u64> = state
        .versions
        .iter()
        .map(|version| version.revision)
        .collect();
    transaction.execute(
        "DELETE FROM session_boundaries WHERE node_id = ?1",
        [node.as_str()],
    )?;
    transaction.execute(
        "DELETE FROM command_snapshots WHERE node_id = ?1 AND revision NOT IN (SELECT value FROM json_each(?2))",
        params![node.as_str(), to_json(&revisions)],
    )?;
    for boundary in &state.boundaries {
        transaction.execute(
            "INSERT INTO session_boundaries (node_id, block_id, edge, command_revision) VALUES (?1, ?2, ?3, ?4)",
            params![
                node.as_str(),
                boundary.block_id.as_str(),
                edge_name(boundary.edge),
                integer(boundary.command_revision)
            ],
        )?;
    }
    Ok(Ok(()))
}

/// A node's checkpoint: its state row, every block row below its block
/// count, and its command state; none when the node does not exist.
fn checkpoint(connection: &Connection, node: &NodeId) -> Result<Option<Checkpoint>, StorageError> {
    let row: Option<(String, i64, i64)> = connection
        .query_row(
            "SELECT state, block_count, command_revision FROM nodes WHERE id = ?1",
            [node.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((state, block_count, revision)) = row else {
        return Ok(None);
    };
    Ok(Some(Checkpoint {
        state: json("nodes", "state", &state)?,
        transcript: blocks_of(connection, node, block_count)?,
        command_state: command_state(connection, node, revision)?,
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

/// A node's command state: every version and every boundary, with the
/// current revision the node row names.
fn command_state(
    connection: &Connection,
    node: &NodeId,
    revision: i64,
) -> Result<CommandStateSnapshot, StorageError> {
    let mut versions = Vec::new();
    let mut statement = connection.prepare(
        "SELECT revision, entries FROM command_snapshots WHERE node_id = ?1 ORDER BY revision",
    )?;
    let mut rows = statement.query([node.as_str()])?;
    while let Some(row) = rows.next()? {
        let entries: String = row.get(1)?;
        versions.push(CommandVersion {
            revision: unsigned(row, "command_snapshots", "revision")?,
            values: version_values(&entries)?,
        });
    }
    let mut boundaries = Vec::new();
    let mut statement = connection.prepare(
        "SELECT block_id, edge, command_revision FROM session_boundaries WHERE node_id = ?1 ORDER BY block_id, edge",
    )?;
    let mut rows = statement.query([node.as_str()])?;
    while let Some(row) = rows.next()? {
        let edge: String = row.get("edge")?;
        boundaries.push(SessionBoundary {
            block_id: decode(
                "session_boundaries",
                "block_id",
                BlockId::try_from(row.get::<_, String>("block_id")?),
            )?,
            edge: decode(
                "session_boundaries",
                "edge",
                serde_json::from_value(Value::String(edge)),
            )?,
            command_revision: unsigned(row, "session_boundaries", "command_revision")?,
        });
    }
    Ok(CommandStateSnapshot {
        revision: decode("nodes", "command_revision", u64::try_from(revision))?,
        versions,
        boundaries,
    })
}

fn version_values(entries: &str) -> Result<BTreeMap<CommandStorageKey, Value>, StorageError> {
    decode(
        "command_snapshots",
        "entries",
        serde_json::from_str(entries),
    )
}

fn unsigned(row: &Row<'_>, table: &'static str, column: &'static str) -> Result<u64, StorageError> {
    decode(table, column, u64::try_from(row.get::<_, i64>(column)?))
}

/// A boundary edge as its column holds it: the edge's wire name.
fn edge_name(edge: BoundaryEdge) -> String {
    match serde_json::to_value(edge) {
        Ok(Value::String(name)) => name,
        // A unit variant of a string enum serializes as its name.
        _ => unreachable!("a boundary edge serializes as a string"),
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
    "id, number, parent_id, description, profile, round, started_at, can_spawn, closed_phase,
     closed_at, result, failure, delivered";

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
    Ok(NodeRecord {
        id: decode(TABLE, "id", NodeId::try_from(row.get::<_, String>("id")?))?,
        number: decode(TABLE, "number", u64::try_from(row.get::<_, i64>("number")?))?,
        parent: parent
            .map(|parent| decode(TABLE, "parent_id", NodeId::try_from(parent)))
            .transpose()?,
        description: row.get("description")?,
        profile: row.get("profile")?,
        round: decode(TABLE, "round", u64::try_from(row.get::<_, i64>("round")?))?,
        started_at: instant(row, TABLE, "started_at")?,
        can_spawn_subagents: row.get("can_spawn")?,
        closed,
        delivered: row.get("delivered")?,
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
/// state and transactions): the root's phase and output revision and the
/// kind of its latest terminal block, read without loading the transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SummaryFacts {
    pub phase: SessionPhase,
    pub revision: u64,
    pub last: Option<Terminal>,
}

impl SummaryFacts {
    /// The facts of a conversation that has no tree yet: idle, revision 0.
    pub const EMPTY: Self = Self {
        phase: SessionPhase::Idle,
        revision: 0,
        last: None,
    };
}

/// How the root's latest finished request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terminal {
    Response,
    Error,
    Abort,
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
    let Some((id, state, block_count, revision)) = root else {
        return Ok(SummaryFacts::EMPTY);
    };
    let state: CheckpointState = json("nodes", "state", &state)?;
    let terminal: Option<String> = connection
        .query_row(
            "SELECT block FROM blocks WHERE node_id = ?1 AND idx < ?2
               AND json_extract(block, '$.type') IN ('response', 'error', 'abort')
             ORDER BY idx DESC LIMIT 1",
            params![id, block_count],
            |row| row.get(0),
        )
        .optional()?;
    let last = match terminal {
        None => None,
        Some(text) => match json::<Block>("blocks", "block", &text)? {
            Block::Response(_) => Some(Terminal::Response),
            Block::Error(_) => Some(Terminal::Error),
            Block::Abort(_) => Some(Terminal::Abort),
            _ => unreachable!("the query selects only terminal blocks"),
        },
    };
    Ok(SummaryFacts {
        phase: state.phase,
        revision: decode("nodes", "output_revision", u64::try_from(revision))?,
        last,
    })
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

/// A tree's history as its database holds it: the root's blocks, and each
/// subagent's record with its blocks, depth first in spawn order.
#[derive(Debug, Default)]
pub struct History {
    pub blocks: Vec<Block>,
    pub subagents: Vec<(NodeRecord, Vec<Block>)>,
}

/// The history of the tree `connection` holds; an empty one when it has no
/// root yet.
pub fn history(connection: &Connection) -> Result<History, StorageError> {
    let root: Option<(String, i64)> = connection
        .query_row(
            "SELECT id, block_count FROM nodes WHERE parent_id IS NULL",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((root, block_count)) = root else {
        return Ok(History::default());
    };
    let root = decode("nodes", "id", NodeId::try_from(root))?;
    let blocks = blocks_of(connection, &root, block_count)?;
    let mut children: HashMap<NodeId, Vec<NodeRecord>> = HashMap::new();
    let mut statement = connection.prepare(&format!(
        "SELECT {NODE_COLUMNS} FROM nodes WHERE parent_id IS NOT NULL ORDER BY number"
    ))?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let node = node_row(row)?;
        let parent = node
            .parent
            .clone()
            .expect("the query selects nodes with a parent");
        children.entry(parent).or_default().push(node);
    }
    let mut subagents = Vec::new();
    let mut pending: Vec<NodeRecord> = children
        .remove(&root)
        .unwrap_or_default()
        .into_iter()
        .rev()
        .collect();
    while let Some(node) = pending.pop() {
        let block_count: i64 = connection.query_row(
            "SELECT block_count FROM nodes WHERE id = ?1",
            [node.id.as_str()],
            |row| row.get(0),
        )?;
        let blocks = blocks_of(connection, &node.id, block_count)?;
        pending.extend(
            children
                .remove(&node.id)
                .unwrap_or_default()
                .into_iter()
                .rev(),
        );
        subagents.push((node, blocks));
    }
    Ok(History { blocks, subagents })
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;
    use std::sync::Mutex;

    use demi_agent_store::testing::{store_contract, test_model, text};
    use demi_agent_transcript::retire::Retirement;
    use demi_shared_types::{
        B64Bytes, BlobRef, EditCopies, EditKind, EditSegment, EditedFile, MediaSource,
        ResponseBlock, ShellId, ShellToolView, ShellViewStatus, TextBlock, TokenUsage,
        ToolCallBlock, ToolCallStatus, ToolMediaSource, ToolResultContentBlock, ToolView, TurnId,
        UserBlock, UserContentBlock,
    };
    use demi_web_api_protocol::ids::ConversationId;

    use super::*;
    use crate::conversations::ConversationStores;

    /// The owner's blobs in memory, named by their SHA-256 as the object
    /// store names them, with a record of uses that refuses no commit.
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

    impl OwnerBlobs for Blobs {
        fn media(&self) -> &dyn BlobStore {
            self
        }

        fn commit_uses(&self, _: &[BlobRef]) -> Result<(), StoreError> {
            Ok(())
        }
    }

    /// A tree store over a new conversation database and its owner's blob
    /// namespace, with the stores that hold it.
    async fn store() -> (SqliteTreeStore, ConversationStores, tempfile::TempDir) {
        let data = tempfile::tempdir().unwrap();
        let stores = ConversationStores::open(
            data.path().join("conversations"),
            NonZeroUsize::new(4).unwrap(),
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

    fn conversation() -> ConversationId {
        ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b").unwrap()
    }

    fn id(value: &str) -> NodeId {
        NodeId::try_from(value).unwrap()
    }

    /// A node in its first round, agent `number` of the conversation.
    fn record(node: &str, parent: Option<&str>, number: u64) -> NodeRecord {
        NodeRecord {
            id: id(node),
            number,
            parent: parent.map(id),
            description: format!("{node} task"),
            profile: None,
            round: 1,
            started_at: Timestamp::UNIX_EPOCH,
            can_spawn_subagents: true,
            closed: None,
            delivered: false,
        }
    }

    fn state() -> CheckpointState {
        CheckpointState {
            phase: SessionPhase::Idle,
            queue: Vec::new(),
            agent_inputs: Vec::new(),
            wakeups: Vec::new(),
            cwd: "/w".into(),
            model: test_model(),
            harness: "test".into(),
            edits: Vec::new(),
        }
    }

    fn update(blocks: Vec<(usize, Block)>, block_count: usize) -> CheckpointUpdate {
        CheckpointUpdate {
            state: state(),
            command_state: None,
            changed_blocks: blocks,
            block_count,
        }
    }

    fn user(block: &str) -> Block {
        Block::User(UserBlock {
            id: block.try_into().unwrap(),
            turn_id: TurnId::try_from("t1").unwrap(),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            content: text("hi"),
            preamble: None,
        })
    }

    fn reply(block: &str) -> Block {
        Block::Text(TextBlock {
            id: block.try_into().unwrap(),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            text: "hello".into(),
            forkable: false,
        })
    }

    fn response(block: &str) -> Block {
        Block::Response(ResponseBlock {
            id: block.try_into().unwrap(),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            usage: TokenUsage::default(),
        })
    }

    async fn facts(stores: &ConversationStores) -> SummaryFacts {
        stores
            .read(&conversation(), summary)
            .await
            .unwrap()
            .unwrap()
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

        root.save(update(vec![(0, user("u1"))], 1), &CommitGuard::default())
            .await
            .unwrap();
        assert_eq!(facts(&stores).await.revision, 0, "the user's message alone");
        root.save(
            update(vec![(1, reply("a1")), (2, response("r1"))], 3),
            &CommitGuard::default(),
        )
        .await
        .unwrap();
        let answered = facts(&stores).await;
        assert_eq!(
            (answered.revision, answered.last),
            (1, Some(Terminal::Response))
        );
        // A rewrite that drops rows is a change of the output.
        root.save(update(Vec::new(), 1), &CommitGuard::default())
            .await
            .unwrap();
        let rewritten = facts(&stores).await;
        assert_eq!((rewritten.revision, rewritten.last), (2, None));
    }

    #[tokio::test(flavor = "local")]
    async fn a_refused_save_leaves_the_whole_checkpoint_as_it_was() {
        let (tree, _stores, _data) = store().await;
        tree.create_node(record("root", None, 0), update(Vec::new(), 0))
            .await
            .unwrap();
        let root = tree.session_store(&id("root"));
        let before = root.load().await.unwrap().unwrap();

        // A version the store holds is immutable: the save that changes it
        // writes neither its blocks nor its state.
        let mut changed = CommandStateSnapshot::initial();
        changed.versions[0].values.insert(
            "todos.json".to_owned().try_into().unwrap(),
            Value::Bool(true),
        );
        let mut save = update(vec![(0, user("u1"))], 1);
        save.command_state = Some(changed);
        save.state.phase = SessionPhase::Running;
        let refused = root.save(save, &CommitGuard::default()).await.unwrap_err();
        assert_eq!(
            refused,
            StoreError::Failed("command-state version 0 is immutable".into())
        );
        assert_eq!(root.load().await.unwrap().unwrap(), before);

        // A save serving an invocation a rewrite made stale commits nothing.
        let stale = tokio_util::sync::CancellationToken::new();
        stale.cancel();
        let refused = root
            .save(
                update(vec![(0, user("u1"))], 1),
                &CommitGuard::new(vec![stale]),
            )
            .await
            .unwrap_err();
        assert_eq!(refused, StoreError::Invalidated);
        assert_eq!(root.load().await.unwrap().unwrap(), before);
    }

    #[tokio::test(flavor = "local")]
    async fn the_history_is_the_root_then_each_subagent_depth_first_in_spawn_order() {
        let (tree, stores, _data) = store().await;
        tree.create_node(record("root", None, 0), update(vec![(0, user("u1"))], 1))
            .await
            .unwrap();
        for (node, parent, round) in [("b", "root", 5), ("a", "root", 3), ("a1", "a", 4)] {
            let blocks = vec![(0, reply(&format!("{node}-text")))];
            tree.create_node(record(node, Some(parent), round), update(blocks, 1))
                .await
                .unwrap();
        }

        let history = stores
            .read(&conversation(), history)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(history.blocks, [user("u1")]);
        let order: Vec<(&str, &Block)> = history
            .subagents
            .iter()
            .map(|(node, blocks)| (node.id.as_str(), &blocks[0]))
            .collect();
        assert_eq!(
            order,
            [
                ("a", &reply("a-text")),
                ("a1", &reply("a1-text")),
                ("b", &reply("b-text"))
            ]
        );
        assert_eq!(history.subagents[0].0, record("a", Some("root"), 3));
    }

    /// A shell call written at `at`, whose result holds the images that
    /// `images` name.
    fn shot(block: &str, at: Timestamp, images: &[u8]) -> Block {
        let output = images
            .iter()
            .map(|byte| ToolResultContentBlock::Image {
                source: ToolMediaSource::Ref {
                    r#ref: BlobRef::of(&[*byte]),
                    media_type: "image/png".into(),
                },
            })
            .collect();
        Block::ToolCall(ToolCallBlock {
            id: block.try_into().unwrap(),
            created_at: at,
            model: test_model(),
            tool_use_id: format!("toolu_{block}"),
            tool_name: "shell_exec".into(),
            input: "{}".into(),
            status: ToolCallStatus::Completed,
            output,
            view: None,
        })
    }

    /// A shell call written at `at` whose command edited one file in
    /// segments, each side a blob that one of `sides` names: segment `n` goes
    /// from side `n` to side `n + 1`.
    fn edited(block: &str, at: Timestamp, sides: &[u8]) -> Block {
        let edits = sides
            .windows(2)
            .map(|pair| EditSegment {
                copies: Some(EditCopies {
                    original: BlobRef::of(&[pair[0]]),
                    modified: BlobRef::of(&[pair[1]]),
                }),
            })
            .collect();
        Block::ToolCall(ToolCallBlock {
            id: block.try_into().unwrap(),
            created_at: at,
            model: test_model(),
            tool_use_id: format!("toolu_{block}"),
            tool_name: "shell_exec".into(),
            input: "{}".into(),
            status: ToolCallStatus::Completed,
            output: Vec::new(),
            view: Some(ToolView::Shell(ShellToolView {
                status: ShellViewStatus::Exited,
                shell_id: ShellId::try_from("shell-1").unwrap(),
                command_id: CommandId::try_from(format!("command-{block}")).unwrap(),
                exit_code: Some(0),
                running_ms: 1,
                idle_ms: 0,
                chunks: Vec::new(),
                view_truncated: false,
                files: Some(vec![EditedFile {
                    path: "/work/notes.md".into(),
                    kind: EditKind::Modified,
                    added: 1,
                    removed: 1,
                    edits,
                }]),
                files_truncated: Some(false),
            })),
        })
    }

    /// A message written at `at` with the uploaded image `image` names.
    fn pasted(block: &str, at: Timestamp, image: u8) -> Block {
        Block::User(UserBlock {
            id: block.try_into().unwrap(),
            turn_id: TurnId::try_from(block).unwrap(),
            created_at: at,
            model: test_model(),
            content: vec![UserContentBlock::Image {
                source: MediaSource::Ref {
                    r#ref: BlobRef::of(&[image]),
                    media_type: "image/png".into(),
                },
            }],
            preamble: None,
        })
    }

    /// An index row as the `blob_refs` table holds it: the node, the block's
    /// index, the reference's place, the blob, its holder and the block's
    /// time.
    type Indexed = Vec<(String, i64, i64, String, String, i64)>;

    /// The rows the `blob_refs` table holds, and the rows the one derivation
    /// makes of the blocks, in the same order.
    fn index(connection: &Connection) -> Result<(Indexed, Indexed), StorageError> {
        let mut held = Vec::new();
        let mut statement = connection
            .prepare("SELECT node_id, idx, part, blob, holder, at FROM blob_refs ORDER BY node_id, idx, part")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            held.push((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ));
        }
        let mut derived = Vec::new();
        let mut statement =
            connection.prepare("SELECT node_id, idx, block FROM blocks ORDER BY node_id, idx")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let (node, index): (String, i64) = (row.get(0)?, row.get(1)?);
            let block: Block = json("blocks", "block", &row.get::<_, String>(2)?)?;
            derived.extend(blob_refs::rows(&block).into_iter().map(|row| {
                (
                    node.clone(),
                    index,
                    count(row.part),
                    row.blob.to_string(),
                    blob_refs::holder_name(row.holder).to_owned(),
                    row.at.as_millisecond(),
                )
            }));
        }
        Ok((held, derived))
    }

    #[tokio::test(flavor = "local")]
    async fn every_write_of_a_block_keeps_the_blob_index_what_the_blocks_derive() {
        let (tree, stores, _data) = store().await;
        let check = async |path: &str| {
            let (held, derived) = stores.read(&conversation(), index).await.unwrap().unwrap();
            assert!(
                !held.is_empty() || path == "an edit",
                "after {path}: the index holds rows"
            );
            assert_eq!(held, derived, "after {path}");
        };
        let written = Timestamp::UNIX_EPOCH;
        // A Fork's seed is the first checkpoint of its root, with a history.
        let seed = vec![
            (0, pasted("u1", written, 1)),
            (1, shot("t1", written, &[2, 3])),
        ];
        tree.create_node(record("root", None, 0), update(seed, 2))
            .await
            .unwrap();
        check("a Fork's seed").await;
        let root = tree.session_store(&id("root"));
        let save = async |blocks: Vec<(usize, Block)>, count: usize| {
            root.save(update(blocks, count), &CommitGuard::default())
                .await
                .unwrap();
        };
        save(vec![(2, shot("t2", written, &[4])), (3, reply("a1"))], 4).await;
        check("a save").await;
        // A history rewrite writes blocks anew in place and drops the rest.
        save(vec![(1, reply("a2")), (2, shot("t3", written, &[5, 6]))], 3).await;
        check("a history rewrite").await;
        // An edit replaces the message and drops everything after it.
        save(vec![(0, pasted("u2", written, 7))], 1).await;
        check("an edit").await;
        // A command's edit copies: the middle side is both segments'.
        save(
            vec![
                (1, shot("t4", written, &[8])),
                (2, edited("e1", written, &[10, 11, 12])),
            ],
            3,
        )
        .await;
        check("a save of edit copies").await;
        let child = vec![(0, shot("c1", written, &[9]))];
        tree.create_node(record("child", Some("root"), 2), update(child, 1))
            .await
            .unwrap();
        tree.delete_node(&id("child")).await.unwrap();
        check("a subagent's deletion").await;

        // Forty days later the conversation has been idle for thirty.
        let before = facts(&stores).await;
        let retirement = Retirement {
            now: Timestamp::from_millisecond(40 * 24 * 60 * 60 * 1000).unwrap(),
            idle: true,
        };
        let retired = stores
            .db(&conversation())
            .call(move |connection| blob_refs::retire(connection, &Blobs::default(), retirement))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retired, 1);
        check("a retirement").await;
        // The message's image and the edit copies stay; the retirement shows
        // the page nothing new.
        let (held, _) = stores.read(&conversation(), index).await.unwrap().unwrap();
        let kept: Vec<(i64, &str)> = held.iter().map(|row| (row.1, row.4.as_str())).collect();
        assert_eq!(
            kept,
            [
                (0, "message"),
                (2, "edit_copy"),
                (2, "edit_copy"),
                (2, "edit_copy"),
                (2, "edit_copy")
            ]
        );
        assert_eq!(facts(&stores).await, before);
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
