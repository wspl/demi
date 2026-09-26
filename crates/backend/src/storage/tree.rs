//! The tree store over a conversation's database (`runtime.md` § Tree store,
//! `storage.md` § Conversation state and transactions): a node's record is
//! its `nodes` row, and its checkpoint is that row's state with the node's
//! `blocks` rows and its command state (`command_snapshots`,
//! `session_boundaries`). Creating, saving, closing, reopening and deleting
//! a node are each one transaction on the conversation's writer connection,
//! where rows are also serialized and decoded. Every row read is decoded and
//! checked, and corrupt data stops the read.
//!
//! A block's inline media goes to the conversation owner's blob namespace
//! before the save that references it, and comes back when a session loads
//! for inference (`storage.md` § Attachment and transcript media).
//!
//! The same readings serve what the browser reads without a live session: a
//! conversation's summary facts and its history, on a read-only connection,
//! which keep the media references.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use demi_agent::store::{
    BoundaryEdge, Checkpoint, CheckpointState, CheckpointUpdate, ClosePhase, CommandStateSnapshot, CommandStorageKey,
    CommandVersion, CommitGuard, NodeClose, NodeRecord, SessionBoundary, StoreError, media,
};
use demi_agent::{AgentTreeStore, SessionStore};
use demi_core::{Block, BlockId, CompletionId, NodeId, QueuedMessage, SessionPhase, Timestamp};
use futures_util::future::LocalBoxFuture;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use serde_json::Value;

use super::StorageError;
use super::blobs::UserBlobs;
use super::columns::{decode, json, to_json};
use super::conversations::ConversationDb;

/// One conversation's agent tree in its database, with its owner's blobs.
pub(crate) struct SqliteTreeStore {
    db: ConversationDb,
    blobs: UserBlobs,
}

impl SqliteTreeStore {
    pub(crate) fn new(db: ConversationDb, blobs: UserBlobs) -> Self {
        Self { db, blobs }
    }
}

/// One node's checkpoint in its conversation's database.
struct SqliteSessionStore {
    db: ConversationDb,
    blobs: UserBlobs,
    node: NodeId,
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
    fn node<'a>(&'a self, id: &'a NodeId) -> LocalBoxFuture<'a, Result<Option<NodeRecord>, StoreError>> {
        let id = id.clone();
        Box::pin(async move {
            self.db
                .call(move |connection| node_by_id(connection, &id))
                .await
                .map_err(store_error)
        })
    }

    fn children<'a>(&'a self, parent: &'a NodeId) -> LocalBoxFuture<'a, Result<Vec<NodeRecord>, StoreError>> {
        let parent = parent.clone();
        Box::pin(async move {
            self.db
                .call(move |connection| children_of(connection, &parent))
                .await
                .map_err(store_error)
        })
    }

    fn create_node(&self, record: NodeRecord, initial: CheckpointUpdate) -> LocalBoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            let completions = initial.carried_completions()?;
            let mut initial = initial;
            media::externalize_update(&mut initial, &self.blobs).await?;
            self.db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    if node_by_id(&transaction, &record.id)?.is_some() {
                        return Ok(Err(StoreError::Failed(format!("node {} already exists", record.id))));
                    }
                    let (phase, closed_at, result, failure) = close_columns(record.closed.as_ref());
                    transaction.execute(
                        "INSERT INTO nodes (id, parent_id, description, profile, spawned_at, can_spawn, closed_phase,
                           closed_at, result, failure, delivered, state, block_count, command_revision, output_revision)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0, 0, 0)",
                        params![
                            record.id.as_str(),
                            record.parent.as_ref().map(NodeId::as_str),
                            record.description,
                            record.profile,
                            millis(record.round),
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
                    let written = write_checkpoint(&transaction, &record.id, &initial, &completions)?;
                    if written.is_ok() {
                        transaction.commit()?;
                    }
                    Ok(written)
                })
                .await
                .map_err(store_error)?
        })
    }

    fn session_store(&self, id: &NodeId) -> Rc<dyn SessionStore> {
        Rc::new(SqliteSessionStore {
            db: self.db.clone(),
            blobs: self.blobs.clone(),
            node: id.clone(),
        })
    }

    fn close_node<'a>(&'a self, id: &'a NodeId, close: NodeClose) -> LocalBoxFuture<'a, Result<(), StoreError>> {
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
                        "UPDATE nodes SET spawned_at = ?2, closed_phase = NULL, closed_at = NULL, result = NULL,
                           failure = NULL, delivered = 0, state = ?3
                         WHERE id = ?1",
                        params![node.as_str(), millis(round), to_json(&state)],
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

    fn mark_delivered<'a>(&'a self, id: &'a NodeId, round: u64) -> LocalBoxFuture<'a, Result<(), StoreError>> {
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
                        "UPDATE nodes SET delivered = 1 WHERE id = ?1 AND spawned_at = ?2",
                        params![node.as_str(), millis(round)],
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
                    connection.execute("DELETE FROM nodes WHERE id = ?1", [node.as_str()])?;
                    Ok(())
                })
                .await
                .map_err(store_error)
        })
    }
}

impl SessionStore for SqliteSessionStore {
    fn save<'a>(&'a self, update: CheckpointUpdate, guard: &'a CommitGuard) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        let guard = guard.clone();
        let node = self.node.clone();
        Box::pin(async move {
            let completions = update.carried_completions()?;
            let mut update = update;
            media::externalize_update(&mut update, &self.blobs).await?;
            let commit = self.db.commit_point();
            self.db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    // The guard is checked at the start of the transaction, so
                    // an invocation a rewrite made stale commits nothing.
                    if let Err(stale) = guard.check() {
                        return Ok(Err(stale));
                    }
                    let written = write_checkpoint(&transaction, &node, &update, &completions)?;
                    if written.is_ok() {
                        commit.commit(transaction)?;
                    }
                    Ok(written)
                })
                .await
                .map_err(store_error)?
        })
    }

    fn load(&self) -> LocalBoxFuture<'_, Result<Option<Checkpoint>, StoreError>> {
        let node = self.node.clone();
        Box::pin(async move {
            let loaded = self
                .db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    checkpoint(&transaction, &node)
                })
                .await
                .map_err(store_error)?;
            let Some(mut checkpoint) = loaded else {
                return Ok(None);
            };
            media::rehydrate_checkpoint(&mut checkpoint, &self.blobs).await?;
            Ok(Some(checkpoint))
        })
    }
}

/// Writes one save of `node` in `transaction`: the changed block rows, the
/// rows past the new end gone, the state row, the command state when it
/// changed, and the child completions it carries marked delivered. Changed
/// output advances the node's output revision; input alone does not. A
/// refusal leaves the transaction uncommitted.
fn write_checkpoint(
    transaction: &Transaction<'_>,
    node: &NodeId,
    update: &CheckpointUpdate,
    completions: &[CompletionId],
) -> Result<Result<(), StoreError>, StorageError> {
    for (index, block) in &update.changed_blocks {
        transaction.execute(
            "INSERT INTO blocks (node_id, idx, block) VALUES (?1, ?2, ?3)
             ON CONFLICT (node_id, idx) DO UPDATE SET block = excluded.block",
            params![node.as_str(), count(*index), to_json(block)],
        )?;
    }
    let block_count = count(update.block_count);
    transaction.execute(
        "DELETE FROM blocks WHERE node_id = ?1 AND idx >= ?2",
        params![node.as_str(), block_count],
    )?;
    let output = update.changed_blocks.iter().any(|(_, block)| is_output(block));
    let revision = update.command_state.as_ref().map(|state| millis(state.revision));
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
            "UPDATE nodes SET delivered = 1 WHERE id = ?1 AND parent_id = ?2 AND spawned_at = ?3",
            params![round.child.as_str(), node.as_str(), millis(round.round)],
        )?;
    }
    Ok(Ok(()))
}

/// Whether a block is output, which the user has not seen until the page
/// shows it: anything but the input blocks, the user's messages and steers
/// and the inputs the session and the tree write.
fn is_output(block: &Block) -> bool {
    !matches!(
        block,
        Block::User(_) | Block::Context(_) | Block::Wakeup(_) | Block::Steer(_) | Block::AgentMessage(_) | Block::Resume(_)
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
                params![node.as_str(), millis(version.revision)],
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
                    params![node.as_str(), millis(version.revision), to_json(&version.values)],
                )?;
            }
        }
    }
    let revisions: Vec<u64> = state.versions.iter().map(|version| version.revision).collect();
    transaction.execute("DELETE FROM session_boundaries WHERE node_id = ?1", [node.as_str()])?;
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
                millis(boundary.command_revision)
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
fn blocks_of(connection: &Connection, node: &NodeId, block_count: i64) -> Result<Vec<Block>, StorageError> {
    let mut statement = connection.prepare("SELECT idx, block FROM blocks WHERE node_id = ?1 AND idx < ?2 ORDER BY idx")?;
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
fn command_state(connection: &Connection, node: &NodeId, revision: i64) -> Result<CommandStateSnapshot, StorageError> {
    let mut versions = Vec::new();
    let mut statement =
        connection.prepare("SELECT revision, entries FROM command_snapshots WHERE node_id = ?1 ORDER BY revision")?;
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
            block_id: decode("session_boundaries", "block_id", BlockId::try_from(row.get::<_, String>("block_id")?))?,
            edge: decode("session_boundaries", "edge", serde_json::from_value(Value::String(edge)))?,
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
    decode("command_snapshots", "entries", serde_json::from_str(entries))
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
fn node_state(connection: &Connection, node: &NodeId) -> Result<Option<CheckpointState>, StorageError> {
    let state: Option<String> = connection
        .query_row("SELECT state FROM nodes WHERE id = ?1", [node.as_str()], |row| row.get(0))
        .optional()?;
    state.map(|state| json("nodes", "state", &state)).transpose()
}

const NODE_COLUMNS: &str =
    "id, parent_id, description, profile, spawned_at, can_spawn, closed_phase, closed_at, result, failure, delivered";

fn node_by_id(connection: &Connection, id: &NodeId) -> Result<Option<NodeRecord>, StorageError> {
    let mut statement = connection.prepare(&format!("SELECT {NODE_COLUMNS} FROM nodes WHERE id = ?1"))?;
    let mut rows = statement.query([id.as_str()])?;
    rows.next()?.map(node_row).transpose()
}

/// A node's direct children in spawn order: by round, then as they were
/// created.
fn children_of(connection: &Connection, parent: &NodeId) -> Result<Vec<NodeRecord>, StorageError> {
    let mut statement =
        connection.prepare(&format!("SELECT {NODE_COLUMNS} FROM nodes WHERE parent_id = ?1 ORDER BY spawned_at, rowid"))?;
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
        parent: parent.map(|parent| decode(TABLE, "parent_id", NodeId::try_from(parent))).transpose()?,
        description: row.get("description")?,
        profile: row.get("profile")?,
        round: round(row)?,
        can_spawn_subagents: row.get("can_spawn")?,
        closed,
        delivered: row.get("delivered")?,
    })
}

/// A node's round: when it started, in milliseconds since the Unix epoch,
/// which must be a time.
fn round(row: &Row<'_>) -> Result<u64, StorageError> {
    let millisecond: i64 = row.get("spawned_at")?;
    decode("nodes", "spawned_at", Timestamp::from_millisecond(millisecond))?;
    decode("nodes", "spawned_at", u64::try_from(millisecond))
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
fn close_columns(close: Option<&NodeClose>) -> (Option<&'static str>, Option<i64>, Option<String>, Option<String>) {
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

/// A count or index as the INTEGER column holds it; no transcript comes near
/// `i64::MAX` rows.
fn count(value: usize) -> i64 {
    i64::try_from(value).expect("a block count fits the column")
}

/// A round or revision as the INTEGER column holds it: rounds are
/// milliseconds since the Unix epoch and revisions count saves, so neither
/// comes near `i64::MAX`.
fn millis(value: u64) -> i64 {
    i64::try_from(value).expect("a round or revision fits the column")
}

/// What a conversation's summary is built from (`storage.md` § Conversation
/// state and transactions): the root's phase and output revision and the
/// kind of its latest terminal block, read without loading the transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SummaryFacts {
    pub(crate) phase: SessionPhase,
    pub(crate) revision: u64,
    pub(crate) last: Option<Terminal>,
}

impl SummaryFacts {
    /// The facts of a conversation that has no tree yet: idle, revision 0.
    pub(crate) const EMPTY: Self = Self {
        phase: SessionPhase::Idle,
        revision: 0,
        last: None,
    };
}

/// How the root's latest finished request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Terminal {
    Response,
    Error,
    Abort,
}

/// The summary facts of the tree `connection` holds; the empty facts when it
/// has no root yet.
pub(crate) fn summary(connection: &Connection) -> Result<SummaryFacts, StorageError> {
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
pub(crate) fn has_root(connection: &Connection) -> Result<bool, StorageError> {
    let exists = connection.query_row("SELECT EXISTS (SELECT 1 FROM nodes WHERE parent_id IS NULL)", [], |row| {
        row.get(0)
    })?;
    Ok(exists)
}

/// A tree's history as its database holds it: the root's blocks, and each
/// subagent's record with its blocks, depth first in spawn order.
#[derive(Debug, Default)]
pub(crate) struct History {
    pub(crate) blocks: Vec<Block>,
    pub(crate) subagents: Vec<(NodeRecord, Vec<Block>)>,
}

/// The history of the tree `connection` holds; an empty one when it has no
/// root yet.
pub(crate) fn history(connection: &Connection) -> Result<History, StorageError> {
    let root: Option<(String, i64)> = connection
        .query_row("SELECT id, block_count FROM nodes WHERE parent_id IS NULL", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .optional()?;
    let Some((root, block_count)) = root else {
        return Ok(History::default());
    };
    let root = decode("nodes", "id", NodeId::try_from(root))?;
    let blocks = blocks_of(connection, &root, block_count)?;
    let mut children: HashMap<NodeId, Vec<NodeRecord>> = HashMap::new();
    let mut statement =
        connection.prepare(&format!("SELECT {NODE_COLUMNS} FROM nodes WHERE parent_id IS NOT NULL ORDER BY spawned_at, rowid"))?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let node = node_row(row)?;
        let parent = node.parent.clone().expect("the query selects nodes with a parent");
        children.entry(parent).or_default().push(node);
    }
    let mut subagents = Vec::new();
    let mut pending: Vec<NodeRecord> = children.remove(&root).unwrap_or_default().into_iter().rev().collect();
    while let Some(node) = pending.pop() {
        let block_count: i64 = connection.query_row("SELECT block_count FROM nodes WHERE id = ?1", [node.id.as_str()], |row| {
            row.get(0)
        })?;
        let blocks = blocks_of(connection, &node.id, block_count)?;
        pending.extend(children.remove(&node.id).unwrap_or_default().into_iter().rev());
        subagents.push((node, blocks));
    }
    Ok(History { blocks, subagents })
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use std::sync::Arc;

    use demi_agent::testing::{store_contract, test_model, text};
    use demi_core::{B64Bytes, BlobRef, MediaSource, ResponseBlock, TextBlock, TokenUsage, TurnId, UserBlock, UserContentBlock};
    use demi_web_api::ids::ConversationId;

    use super::*;
    use crate::storage::blobs::BlobStores;
    use crate::storage::conversations::ConversationStores;
    use crate::storage::objects;
    use crate::storage::objects::fake_s3::FakeS3;

    /// The owner of the conversation the tests store.
    const OWNER: &str = "ana";

    /// A tree store over a new conversation database and its owner's blob
    /// namespace, with the stores that hold it.
    async fn store() -> (SqliteTreeStore, ConversationStores, tempfile::TempDir) {
        let data = tempfile::tempdir().unwrap();
        let stores = ConversationStores::open(data.path().join("conversations"), NonZeroUsize::new(4).unwrap())
            .await
            .unwrap();
        let blobs = BlobStores::new(objects::open(data.path(), None).await.unwrap());
        let owner = demi_web_api::ids::UserId::try_from(OWNER).unwrap();
        (SqliteTreeStore::new(stores.db(&conversation()), blobs.for_user(&owner)), stores, data)
    }

    fn conversation() -> ConversationId {
        ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b").unwrap()
    }

    fn id(value: &str) -> NodeId {
        NodeId::try_from(value).unwrap()
    }

    fn record(node: &str, parent: Option<&str>, round: u64) -> NodeRecord {
        NodeRecord {
            id: id(node),
            parent: parent.map(id),
            description: format!("{node} task"),
            profile: None,
            round,
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
        stores.read(&conversation(), summary).await.unwrap().unwrap()
    }

    #[tokio::test(flavor = "local")]
    async fn output_advances_the_revision_and_input_alone_does_not() {
        let (tree, stores, _data) = store().await;
        assert_eq!(stores.read(&conversation(), summary).await.unwrap(), None);
        tree.create_node(record("root", None, 1), update(Vec::new(), 0)).await.unwrap();
        let root = tree.session_store(&id("root"));
        assert_eq!(facts(&stores).await, SummaryFacts::EMPTY);

        root.save(update(vec![(0, user("u1"))], 1), &CommitGuard::default()).await.unwrap();
        assert_eq!(facts(&stores).await.revision, 0, "the user's message alone");
        root.save(update(vec![(1, reply("a1")), (2, response("r1"))], 3), &CommitGuard::default())
            .await
            .unwrap();
        let answered = facts(&stores).await;
        assert_eq!((answered.revision, answered.last), (1, Some(Terminal::Response)));
        // A rewrite that drops rows is a change of the output.
        root.save(update(Vec::new(), 1), &CommitGuard::default()).await.unwrap();
        let rewritten = facts(&stores).await;
        assert_eq!((rewritten.revision, rewritten.last), (2, None));
    }

    #[tokio::test(flavor = "local")]
    async fn a_refused_save_leaves_the_whole_checkpoint_as_it_was() {
        let (tree, _stores, _data) = store().await;
        tree.create_node(record("root", None, 1), update(Vec::new(), 0)).await.unwrap();
        let root = tree.session_store(&id("root"));
        let before = root.load().await.unwrap().unwrap();

        // A version the store holds is immutable: the save that changes it
        // writes neither its blocks nor its state.
        let mut changed = CommandStateSnapshot::initial();
        changed.versions[0]
            .values
            .insert("todos.json".to_owned().try_into().unwrap(), Value::Bool(true));
        let mut save = update(vec![(0, user("u1"))], 1);
        save.command_state = Some(changed);
        save.state.phase = SessionPhase::Running;
        let refused = root.save(save, &CommitGuard::default()).await.unwrap_err();
        assert_eq!(refused, StoreError::Failed("command-state version 0 is immutable".into()));
        assert_eq!(root.load().await.unwrap().unwrap(), before);

        // A save serving an invocation a rewrite made stale commits nothing.
        let stale = tokio_util::sync::CancellationToken::new();
        stale.cancel();
        let refused = root
            .save(update(vec![(0, user("u1"))], 1), &CommitGuard::new(vec![stale]))
            .await
            .unwrap_err();
        assert_eq!(refused, StoreError::Invalidated);
        assert_eq!(root.load().await.unwrap().unwrap(), before);
    }

    /// A message of the user's with a picture of `bytes`.
    fn picture(block: &str, bytes: &[u8]) -> Block {
        let mut message = user(block);
        let Block::User(user) = &mut message else {
            unreachable!()
        };
        user.content.push(UserContentBlock::Image {
            source: MediaSource::Binary {
                data: B64Bytes::new(bytes.to_vec()),
                media_type: "image/png".into(),
            },
        });
        message
    }

    #[tokio::test(flavor = "local")]
    async fn a_save_whose_media_cannot_be_stored_leaves_the_checkpoint_and_its_media_readable() {
        let s3 = FakeS3::start().await;
        let data = tempfile::tempdir().unwrap();
        let stores = ConversationStores::open(data.path().join("conversations"), NonZeroUsize::new(4).unwrap())
            .await
            .unwrap();
        let owner = demi_web_api::ids::UserId::try_from(OWNER).unwrap();
        let blobs = BlobStores::new(Arc::new(s3.client())).for_user(&owner);
        let tree = SqliteTreeStore::new(stores.db(&conversation()), blobs);
        tree.create_node(record("root", None, 1), update(vec![(0, picture("u1", &[1, 2, 3]))], 1))
            .await
            .unwrap();
        let root = tree.session_store(&id("root"));
        let before = root.load().await.unwrap().unwrap();
        assert_eq!(before.transcript[0], picture("u1", &[1, 2, 3]), "the picture comes back from its blob");

        // The bucket refuses writes: the save that brings a new picture
        // changes nothing, and the picture already stored still reads.
        s3.refuse_puts();
        let save = update(vec![(0, picture("u1", &[1, 2, 3])), (1, picture("u2", &[4, 5, 6]))], 2);
        let refused = root.save(save, &CommitGuard::default()).await.unwrap_err();
        assert!(matches!(refused, StoreError::Failed(_)), "{refused:?}");
        assert_eq!(root.load().await.unwrap().unwrap(), before);
    }

    #[tokio::test(flavor = "local")]
    async fn the_history_is_the_root_then_each_subagent_depth_first_in_spawn_order() {
        let (tree, stores, _data) = store().await;
        tree.create_node(record("root", None, 1), update(vec![(0, user("u1"))], 1)).await.unwrap();
        for (node, parent, round) in [("b", "root", 5), ("a", "root", 3), ("a1", "a", 4)] {
            let blocks = vec![(0, reply(&format!("{node}-text")))];
            tree.create_node(record(node, Some(parent), round), update(blocks, 1)).await.unwrap();
        }

        let history = stores.read(&conversation(), history).await.unwrap().unwrap();
        assert_eq!(history.blocks, [user("u1")]);
        let order: Vec<(&str, &Block)> = history
            .subagents
            .iter()
            .map(|(node, blocks)| (node.id.as_str(), &blocks[0]))
            .collect();
        assert_eq!(
            order,
            [("a", &reply("a-text")), ("a1", &reply("a1-text")), ("b", &reply("b-text"))]
        );
        assert_eq!(history.subagents[0].0, record("a", Some("root"), 3));
    }

    #[tokio::test(flavor = "local")]
    async fn the_database_keeps_the_tree_store_contract() {
        let (tree, _stores, _data) = store().await;
        store_contract::create_queues_the_first_message_with_the_node_and_a_save_replaces_it(&tree).await;
        let (tree, _stores, _data) = store().await;
        store_contract::a_save_delivers_the_completions_its_transcript_carries_and_only_those(&tree).await;
        let (tree, _stores, _data) = store().await;
        store_contract::reopen_makes_a_closed_node_live_with_its_message_and_delete_takes_the_subtree(&tree).await;
        let (tree, _stores, _data) = store().await;
        store_contract::a_completion_of_an_earlier_round_marks_the_current_one_undelivered(&tree).await;
        let (tree, _stores, _data) = store().await;
        store_contract::a_save_delivers_a_completion_it_holds_as_waiting_input(&tree).await;
        let (tree, _stores, _data) = store().await;
        store_contract::children_list_in_spawn_order_and_a_close_keeps_its_result(&tree).await;
        let (tree, _stores, data) = store().await;
        // A blob is the file `blobs/<user>/<sha256>` of the object store
        // (`storage.md` § The object store).
        let forget = |blob: &BlobRef| {
            std::fs::remove_file(data.path().join("blobs").join(OWNER).join(blob.as_str())).unwrap();
        };
        store_contract::media_travels_by_reference(&tree, &tree.blobs, &forget).await;
    }
}
