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
use std::rc::Rc;
use std::sync::Arc;

use demi_agent_store::{AgentTreeStore, SessionStore, StoredCommand, StoredOutput};
use demi_agent_store::{
    Checkpoint, CheckpointState, CheckpointUpdate, ClosePhase, NodeClose, NodeRecord, StoreError,
    media::BlobStore,
};
use demi_agent_transcript::is_interruption;
use demi_backend_remote_host::decode_output;
use demi_shared_types::{
    Block, BlockId, CommandEnd, CommandId, CompletionId, NodeId, QueuedMessage, Sequence, SessionPhase,
    Timestamp, is_blank,
};
use futures_util::future::LocalBoxFuture;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use super::StorageError;
use super::columns::{count, decode, instant, json, to_json};
use super::command_outputs::{self, OutputRow};
use super::conversations::ConversationDb;
use super::sequences;

/// What a tree store tells after each commit that writes a node's
/// checkpoint, closes, reopens or deletes a node, with that node and the
/// conversation's earliest saved wakeup after the commit: a conversation's summary reads
/// its root's checkpoint, and the index of conversations keeps the wakeup
/// (`storage.md` § Control records).
pub type Saved = Rc<dyn Fn(&NodeId, Option<WakeupDue>)>;

/// When a saved yield wakeup is due (`runtime.md` § Yield wakeups): at the
/// next start for one whose action had not ended, since a restore starts
/// its wait; otherwise at its due time. The earlier of two is the lesser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WakeupDue {
    AtStart,
    At(Timestamp),
}

impl WakeupDue {
    /// The earliest of the wakeups `state` saves; none when it saves none.
    fn earliest(state: &CheckpointState) -> Option<Self> {
        state
            .wakeups
            .iter()
            .map(|wakeup| wakeup.due_at.map_or(Self::AtStart, Self::At))
            .min()
    }

    /// As a `wakeup_at` column holds it: milliseconds since the Unix epoch,
    /// and 0 for at start, which no wakeup scheduled since is due at.
    pub(crate) fn column(self) -> i64 {
        match self {
            Self::AtStart => 0,
            Self::At(at) => at.as_millisecond(),
        }
    }

    /// A `wakeup_at` column of `table`, decoded.
    pub(crate) fn from_column(table: &'static str, value: i64) -> Result<Self, StorageError> {
        if value == 0 {
            return Ok(Self::AtStart);
        }
        decode(table, "wakeup_at", Timestamp::from_millisecond(value)).map(Self::At)
    }
}

/// The earliest wakeup the live nodes of the tree `connection` holds save:
/// a closed node's never fire, since restore skips it (`subagents.md`
/// § Persistence).
fn earliest_wakeup(connection: &Connection) -> Result<Option<WakeupDue>, StorageError> {
    let earliest: Option<i64> = connection.query_row(
        "SELECT MIN(wakeup_at) FROM nodes WHERE closed_phase IS NULL",
        [],
        |row| row.get(0),
    )?;
    earliest
        .map(|value| WakeupDue::from_column("nodes", value))
        .transpose()
}

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

    fn create_node(
        &self,
        record: NodeRecord,
        initial: CheckpointUpdate,
    ) -> LocalBoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            let completions = initial.carried_completions()?;
            let node = record.id.clone();
            let wakeup = self.db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    if node_by_id(&transaction, &record.id)?.is_some() {
                        return Ok(Err(StoreError::Failed(format!("node {} already exists", record.id))));
                    }
                    let (phase, closed_at, result, failure) = close_columns(record.closed.as_ref());
                    transaction.execute(
                        "INSERT INTO nodes (id, number, parent_id, description, profile, instructions, round, started_at,
                           can_spawn, closed_phase, closed_at, result, failure, delivered, state, block_count,
                           output_revision)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, 0, 0)",
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
                        ],
                    )?;
                    if let Err(refused) = write_checkpoint(&transaction, &record.id, &initial, &completions)? {
                        return Ok(Err(refused));
                    }
                    let wakeup = earliest_wakeup(&transaction)?;
                    transaction.commit()?;
                    Ok(Ok(wakeup))
                })
                .await
                .map_err(store_error)??;
            (self.saved)(&node, wakeup);
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
            let wakeup = self
                .db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    let changed = transaction.execute(
                        "UPDATE nodes SET closed_phase = ?2, closed_at = ?3, result = ?4, failure = ?5, delivered = 0
                         WHERE id = ?1",
                        params![node.as_str(), phase, closed_at, result, failure],
                    )?;
                    if changed == 0 {
                        return Ok(None);
                    }
                    let wakeup = earliest_wakeup(&transaction)?;
                    transaction.commit()?;
                    Ok(Some(wakeup))
                })
                .await
                .map_err(store_error)?;
            let Some(wakeup) = wakeup else {
                return Err(missing(id));
            };
            (self.saved)(id, wakeup);
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
            let wakeup = self
                .db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    let Some(state) = node_state(&transaction, &node)? else {
                        return Ok(None);
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
                    let wakeup = earliest_wakeup(&transaction)?;
                    transaction.commit()?;
                    Ok(Some(wakeup))
                })
                .await
                .map_err(store_error)?;
            let Some(wakeup) = wakeup else {
                return Err(missing(id));
            };
            (self.saved)(id, wakeup);
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
            // through the cascades, and so do the wakeups they saved.
            let wakeup = self
                .db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    transaction.execute("DELETE FROM nodes WHERE id = ?1", [node.as_str()])?;
                    let wakeup = earliest_wakeup(&transaction)?;
                    transaction.commit()?;
                    Ok(wakeup)
                })
                .await
                .map_err(store_error)?;
            (self.saved)(id, wakeup);
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
            let (blob, missing, media) = match row.output {
                OutputRow::Stored {
                    blob,
                    missing,
                    media,
                } => (blob, missing, media),
                OutputRow::NotStored(reason) => {
                    return Ok(Some(StoredCommand {
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
                end: row.end,
                output: StoredOutput::Stored { output, media },
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
            let wakeup = self
                .db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    if let Err(refused) =
                        write_checkpoint(&transaction, &node, &update, &completions)?
                    {
                        return Ok(Err(refused));
                    }
                    let wakeup = earliest_wakeup(&transaction)?;
                    commit.commit(transaction)?;
                    Ok(Ok(wakeup))
                })
                .await
                .map_err(store_error)??;
            (self.saved)(&self.node, wakeup);
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

/// Writes one save of `node` in `transaction`: the changed block rows, the
/// rows past the new end gone, the state row with its earliest wakeup, and
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
        write.execute(params![node.as_str(), count(*index), to_json(block)])?;
    }
    let block_count = count(update.block_count);
    transaction
        .prepare_cached("DELETE FROM blocks WHERE node_id = ?1 AND idx >= ?2")?
        .execute(params![node.as_str(), block_count])?;
    let output = update
        .changed_blocks
        .iter()
        .any(|(_, block)| is_output(block));
    // The old block count is the column's value before the update: rows
    // gone are a rewrite of the output.
    let wakeup = WakeupDue::earliest(&update.state).map(WakeupDue::column);
    // A root whose last turn was interrupted holds its wakeups until the
    // user resumes it (`runtime.md` § Yield wakeups), so none of them fires
    // by itself: one saved under a turn restores interrupted, and one
    // restored so has saved its interruption record. A child resumes its
    // interrupted turn on its own.
    let interrupted = update.state.phase != SessionPhase::Idle
        || ends_with_interruption(transaction, node, update)?;
    let changed = transaction.execute(
        "UPDATE nodes SET state = ?2, block_count = ?3,
           output_revision = output_revision + (CASE WHEN block_count > ?3 OR ?4 THEN 1 ELSE 0 END),
           wakeup_at = (CASE WHEN parent_id IS NULL AND ?6 THEN NULL ELSE ?5 END)
         WHERE id = ?1",
        params![
            node.as_str(),
            to_json(&update.state),
            block_count,
            output,
            wakeup,
            interrupted
        ],
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

/// Whether the transcript `update` leaves `node` with, its block rows
/// written, ends with the record of an interrupted turn.
fn ends_with_interruption(
    transaction: &Transaction<'_>,
    node: &NodeId,
    update: &CheckpointUpdate,
) -> Result<bool, StorageError> {
    let Some(last) = update.block_count.checked_sub(1) else {
        return Ok(false);
    };
    if let Some((index, block)) = update.changed_blocks.last()
        && *index == last
    {
        return Ok(is_interruption(block));
    }
    // A node that does not exist has no row: the update refuses it next.
    let text: Option<String> = transaction
        .query_row(
            "SELECT block FROM blocks WHERE node_id = ?1 AND idx = ?2",
            params![node.as_str(), count(last)],
            |row| row.get(0),
        )
        .optional()?;
    match text {
        Some(text) => Ok(is_interruption(&json("blocks", "block", &text)?)),
        None => Ok(false),
    }
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
     closed_phase, closed_at, result, failure, delivered";

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
        instructions: row.get("instructions")?,
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
/// state and transactions): the root's phase and output revision, the kind
/// of its latest terminal block and its latest ended turn, read without
/// loading the transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryFacts {
    pub phase: SessionPhase,
    pub revision: u64,
    pub last: Option<Terminal>,
    pub last_turn: Option<EndedTurn>,
}

impl SummaryFacts {
    /// The facts of a conversation that has no tree yet: idle, revision 0.
    pub const EMPTY: Self = Self {
        phase: SessionPhase::Idle,
        revision: 0,
        last: None,
        last_turn: None,
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
    let Some((id, state, block_count, revision)) = root else {
        return Ok(SummaryFacts::EMPTY);
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
    use std::collections::BTreeMap;
    use std::num::NonZeroUsize;
    use std::sync::Mutex;

    use demi_agent_store::testing::{store_contract, test_model, text};
    use demi_agent_transcript::{INTERRUPTED_CODE, INTERRUPTED_TURN_MESSAGE};
    use demi_shared_types::{
        B64Bytes, BlobRef, ErrorBlock, ResponseBlock, TextBlock, TokenUsage, TurnId, UserBlock,
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
            Rc::new(|_: &NodeId, _| {}),
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
            instructions: None,
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
            edits: Vec::new(),
            last_turn: demi_agent_store::TurnEnd::Answer,
        }
    }

    fn update(blocks: Vec<(usize, Block)>, block_count: usize) -> CheckpointUpdate {
        CheckpointUpdate {
            state: state(),
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
            entries: Vec::new(),
        })
    }

    fn reply(block: &str) -> Block {
        Block::Text(TextBlock {
            id: block.try_into().unwrap(),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            text: "hello".into(),
            forkable: false,
            entries: Vec::new(),
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

    /// A save whose state saves wakeups due at each of `due`, in
    /// milliseconds, none for one whose action has not ended.
    fn waiting(due: &[Option<i64>]) -> CheckpointUpdate {
        let wakeups = due
            .iter()
            .enumerate()
            .map(|(index, due)| demi_agent_store::ScheduledWakeup {
                id: format!("w{index}").try_into().unwrap(),
                duration_ms: 1_000,
                command_ids: Vec::new(),
                ended: None,
                due_at: due.map(|at| Timestamp::from_millisecond(at).unwrap()),
            })
            .collect();
        CheckpointUpdate {
            state: CheckpointState { wakeups, ..state() },
            ..update(Vec::new(), 0)
        }
    }

    #[tokio::test(flavor = "local")]
    async fn every_commit_tells_the_earliest_wakeup_the_trees_nodes_save() {
        let data = tempfile::tempdir().unwrap();
        let stores = ConversationStores::open(
            data.path().join("conversations"),
            NonZeroUsize::new(4).unwrap(),
        )
        .await
        .unwrap();
        let told = Rc::new(std::cell::RefCell::new(Vec::new()));
        let saved: Saved = {
            let told = told.clone();
            Rc::new(move |_, wakeup| told.borrow_mut().push(wakeup))
        };
        let tree = SqliteTreeStore::new(
            stores.db(&conversation()),
            Arc::new(Blobs::default()),
            saved,
        );
        let at = |ms| Some(WakeupDue::At(Timestamp::from_millisecond(ms).unwrap()));
        tree.create_node(record("root", None, 0), waiting(&[Some(9_000), None]))
            .await
            .unwrap();
        let root = tree.session_store(&id("root"));
        root.save(waiting(&[Some(9_000)])).await.unwrap();
        tree.create_node(record("child", Some("root"), 1), waiting(&[Some(5_000)]))
            .await
            .unwrap();
        root.save(waiting(&[])).await.unwrap();
        tree.delete_node(&id("child")).await.unwrap();
        // A root whose last turn was interrupted holds its wakeups until the
        // user resumes it: one saved under a turn, and one whose transcript
        // ends with the interruption record, written by this save or an
        // earlier one. A child resumes on its own.
        let mut running = waiting(&[Some(9_000)]);
        running.state.phase = SessionPhase::Running;
        root.save(running.clone()).await.unwrap();
        let interruption = Block::Error(ErrorBlock {
            id: "b2".try_into().unwrap(),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            message: INTERRUPTED_TURN_MESSAGE.into(),
            code: Some(INTERRUPTED_CODE.into()),
            diagnostics: None,
            outside_turn: false,
        });
        let recorded = CheckpointUpdate {
            changed_blocks: vec![(0, user("b1")), (1, interruption)],
            block_count: 2,
            ..waiting(&[Some(9_000)])
        };
        root.save(recorded).await.unwrap();
        let unchanged = CheckpointUpdate {
            block_count: 2,
            ..waiting(&[Some(9_000)])
        };
        root.save(unchanged).await.unwrap();
        tree.create_node(record("child", Some("root"), 1), running)
            .await
            .unwrap();
        // A closed node's wakeups never fire, since restore skips it, until
        // a resume makes it live again.
        let close = NodeClose {
            phase: ClosePhase::Aborted,
            at: Timestamp::UNIX_EPOCH,
        };
        tree.close_node(&id("child"), close).await.unwrap();
        let message = QueuedMessage {
            id: TurnId::try_from("t2").unwrap(),
            content: text("again"),
        };
        tree.reopen_node(&id("child"), 2, Timestamp::UNIX_EPOCH, message)
            .await
            .unwrap();
        assert_eq!(
            *told.borrow(),
            [
                Some(WakeupDue::AtStart),
                at(9_000),
                at(5_000),
                at(5_000),
                None,
                None,
                None,
                None,
                at(9_000),
                None,
                at(9_000)
            ]
        );
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
