//! The tree store over a conversation's database: a node's record is its
//! `nodes` row, and its checkpoint is that row's state with the node's
//! `blocks` rows. Creating, saving, closing, reopening and deleting a node
//! are each one transaction on the conversation's writer connection, where
//! rows are also serialized and decoded. Every row read is decoded and
//! checked, and corrupt data stops the read.
//!
//! Blocks hold their media by reference, and a checkpoint that holds media
//! bytes is refused; each node's session reaches the conversation owner's
//! blob namespace through its store (`crates/backend-blobs/src/blobs.rs`).
//!
//! The same readings serve what the web app reads without a live session: a
//! conversation's summary facts and its history, on a read-only connection,
//! which keep the media references.
//!
//! A block's row changes only while the block is produced; once settled it
//! is never rewritten or deleted. An edit, Regenerate, a retry or an unwind
//! adds rows on a new branch and moves the node's current leaf, so `blocks`
//! is a tree of rows, each naming the row it follows. The node's current
//! path (`path`) is derived from them and stored beside them for page reads:
//! with the search index, one of the two stored copies of derived data.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::{ControlFlow, Range};
use std::rc::Rc;
use std::sync::Arc;

use demi_agent_store::{AgentTreeStore, RunningCommand, SessionStore, StoredCommand, StoredOutput, StoredSubagent, Switched};
use demi_agent_store::{
    Checkpoint, CheckpointState, CheckpointUpdate, ChildDirectory, ClosePhase, EarlierRows, Kind, NodeClose, NodeRecord, Reprofile,
    Placement, Rows, SavedVersions, SideOf, StoreError, StoredPrefix, SwitchedBefore, media::BlobStore,
};
use demi_backend_remote_host::decode_output;
use demi_conversation_lane_protocol::{BlockVersions, PAGE_BLOCKS, PageAt, TurnSpeed, page};
use demi_web_api_protocol::conversations::{CompactionSummary, TranscriptQuery};
use demi_shared_types::{
    Block, BlockId, CommandEnd, CommandId, CompletionId, INSTRUCTIONS_SOURCE, InstructionEntry,
    ModelSelection, NodeId, QueuedMessage, Sequence, SessionPhase, Timestamp, ToolCallBlock, TurnId, is_blank,
};
use futures_util::future::LocalBoxFuture;
use demi_web_api_protocol::ids::{ConversationId, ShareToken};
use demi_agent_store::media::block_blobs;
use crate::path::{self, PATH_LENGTH, PATH_ROWS, POSITION, Path, RUN_NODE, Segment};
use crate::shares;
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params, params_from_iter};

use super::StorageError;
use super::columns::{count, decode, instant, json, to_json};
use super::command_outputs::{self, OutputRow};
use super::contents;
use super::records::{self, Codec, Stored, record_of};
use super::conversations::ConversationDb;
use super::search::{IndexedAgent, IndexedMessage, TranscriptChange, searchable_text};
use super::{conversation_attachments, running, sequences};

/// What a tree store tells after each commit that writes a node's checkpoint,
/// closes, reopens or deletes a node, with that node.
///
/// A conversation's summary reads its root's checkpoint. A commit that writes the
/// root's checkpoint also tells what it changed for the search index, extracted on
/// the database's thread, where the decoded blocks stay; the others tell none.
pub type Saved = Rc<dyn Fn(&NodeId, Option<TranscriptChange>)>;

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

    /// Creates the node of `record` with its first checkpoint `initial`, its
    /// path starting as `start` says.
    async fn create(&self, record: NodeRecord, start: Start, initial: CheckpointUpdate) -> Result<(), StoreError> {
        let completions = initial.carried_completions()?;
        let node = record.id.clone();
        let started = !matches!(start, Start::Empty);
        let change = self
            .db
            .call(move |connection| {
                let attached = match &start {
                    Start::Copied(Some(file), _) => {
                        connection.execute("ATTACH DATABASE ?1 AS source", [file.to_string_lossy()])?;
                        true
                    }
                    _ => false,
                };
                let prefix = match &start {
                    Start::Empty => Prefix::None,
                    Start::Copied(file, prefix) => Prefix::Copied(if file.is_some() { "source" } else { "main" }, prefix),
                    Start::Shared(prefix) => Prefix::Shared(prefix),
                };
                let created = create_in(connection, &record, prefix, &initial, &completions);
                if attached {
                    connection.execute("DETACH DATABASE source", [])?;
                }
                // A node that starts with rows is indexed for search whole, as
                // its publication marks it.
                created.map(|created| created.map(|change| change.filter(|_| !started)))
            })
            .await
            .map_err(store_error)??;
        (self.saved)(&node, change);
        Ok(())
    }
}

/// Where a new node's path starts.
enum Start {
    /// Nowhere: the node's first blocks are `initial`'s.
    Empty,
    /// After rows copied as its own from a conversation's database file, or
    /// from this one with none.
    Copied(Option<std::path::PathBuf>, StoredPrefix),
    /// After rows of this conversation's tree that stay their node's, which
    /// the new node's path names.
    Shared(StoredPrefix),
}

/// [`Start`] inside the transaction that creates the node, a copy's rows in
/// the schema it names.
enum Prefix<'a> {
    None,
    Copied(&'a str, &'a StoredPrefix),
    Shared(&'a StoredPrefix),
}

/// The node `record` with its first checkpoint `initial` in one
/// transaction, after the rows of `prefix`.
fn create_in(
    connection: &mut Connection,
    record: &NodeRecord,
    prefix: Prefix<'_>,
    initial: &CheckpointUpdate,
    completions: &[CompletionId],
) -> Result<Result<Option<TranscriptChange>, StoreError>, StorageError> {
    let transaction = connection.transaction()?;
    if node_by_id(&transaction, &record.id)?.is_some() {
        return Ok(Err(StoreError::Failed(format!("node {} already exists", record.id))));
    }
    insert_node(&transaction, record, initial)?;
    let started = match prefix {
        Prefix::None => Ok(()),
        Prefix::Copied(schema, prefix) => copy_prefix(&transaction, &record.id, schema, prefix)?,
        Prefix::Shared(prefix) => share_prefix(&transaction, &record.id, prefix)?,
    };
    if let Err(refused) = started {
        return Ok(Err(refused));
    }
    if let Err(refused) = write_checkpoint(&transaction, &record.id, initial, completions)? {
        return Ok(Err(refused));
    }
    // A new node follows the empty transcript.
    let change = node_version_of(&transaction, &record.id)?.map(|(agent, version)| {
        TranscriptChange::new(agent, SavedVersion::default(), version, &initial.changed_blocks)
    });
    transaction.commit()?;
    Ok(Ok(change))
}

/// Copies the rows of `prefix`, of `schema`'s `blocks`, as the first rows
/// of `node`'s path, each following the one before, with the blobs and
/// attachments they name: the chain of rows from its leaf back to its
/// path's first, so a later switch of the source leaves it as captured.
///
/// The records are copied as they are stored, with the contents they name
/// and the dictionary they were compressed with, so no block is decoded.
fn copy_prefix(
    transaction: &Transaction<'_>,
    node: &NodeId,
    schema: &str,
    prefix: &StoredPrefix,
) -> Result<Result<(), StoreError>, StorageError> {
    let length = count(prefix.length);
    transaction.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS fork_rows (source INTEGER PRIMARY KEY, idx INTEGER NOT NULL);
         CREATE TEMP TABLE IF NOT EXISTS fork_dictionaries (source INTEGER PRIMARY KEY, id INTEGER NOT NULL);
         DELETE FROM temp.fork_rows;
         DELETE FROM temp.fork_dictionaries;",
    )?;
    transaction.execute(
        &format!(
            "WITH RECURSIVE chain (idx, follows, depth) AS (
               SELECT idx, follows, 0 FROM {schema}.blocks WHERE node_id = ?1 AND idx = ?2
               UNION ALL
               SELECT row.idx, row.follows, chain.depth + 1 FROM {schema}.blocks AS row
                 JOIN chain ON row.node_id = ?1 AND row.idx = chain.follows
             )
             INSERT INTO temp.fork_rows (source, idx) SELECT idx, ?3 - 1 - depth FROM chain"
        ),
        params![prefix.node.as_str(), integer(prefix.leaf), length],
    )?;
    let copied: i64 = transaction.query_row("SELECT count(*) FROM temp.fork_rows WHERE idx >= 0", [], |row| row.get(0))?;
    let total: i64 = transaction.query_row("SELECT count(*) FROM temp.fork_rows", [], |row| row.get(0))?;
    if copied != length || total != length {
        return Ok(Err(StoreError::Failed(format!(
            "the prefix of node {} is not {} rows",
            prefix.node, prefix.length
        ))));
    }
    // The dictionaries the copied records were compressed with, which a
    // database of its own receives as its own.
    if schema == "main" {
        transaction.execute("INSERT INTO temp.fork_dictionaries (source, id) SELECT id, id FROM main.dictionaries", [])?;
    } else {
        let used: Vec<i64> = transaction
            .prepare(&format!(
                "SELECT DISTINCT row.dictionary FROM temp.fork_rows AS copy
                   JOIN {schema}.blocks AS row ON row.node_id = ?1 AND row.idx = copy.source
                 WHERE row.dictionary IS NOT NULL ORDER BY row.dictionary"
            ))?
            .query_map([prefix.node.as_str()], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for source in used {
            let id: i64 = transaction.query_row(
                &format!("INSERT INTO main.dictionaries (bytes) SELECT bytes FROM {schema}.dictionaries WHERE id = ?1 RETURNING id"),
                [source],
                |row| row.get(0),
            )?;
            transaction.execute("INSERT INTO temp.fork_dictionaries (source, id) VALUES (?1, ?2)", params![source, id])?;
        }
        transaction.execute(
            &format!(
                "INSERT OR IGNORE INTO main.contents (hash, bytes)
                 SELECT held.hash, held.bytes FROM {schema}.contents AS held
                 WHERE held.id IN (
                   SELECT named.content FROM temp.fork_rows AS copy
                     JOIN {schema}.blocks AS row ON row.node_id = ?1 AND row.idx = copy.source
                     JOIN {schema}.block_contents AS named ON named.record = row.record
                 )"
            ),
            [prefix.node.as_str()],
        )?;
    }
    transaction.execute(
        &format!(
            "INSERT INTO blocks (node_id, idx, follows, replaces, format, type, id, turn_id, command, source, status,
               forkable, placement, created_at, outside_turn, resumed, summary_tokens, dictionary, body)
             SELECT ?1, copy.idx, CASE copy.idx WHEN 0 THEN NULL ELSE copy.idx - 1 END, NULL,
               row.format, row.type, row.id, row.turn_id, row.command, row.source, row.status, row.forkable,
               row.placement, row.created_at, row.outside_turn, row.resumed, row.summary_tokens,
               (SELECT id FROM temp.fork_dictionaries WHERE source = row.dictionary), row.body
             FROM temp.fork_rows AS copy JOIN {schema}.blocks AS row ON row.node_id = ?2 AND row.idx = copy.source
             ORDER BY copy.idx"
        ),
        params![node.as_str(), prefix.node.as_str()],
    )?;
    transaction.execute(
        &format!(
            "INSERT INTO block_contents (record, content)
             SELECT copied.record, held.id
             FROM temp.fork_rows AS copy
               JOIN {schema}.blocks AS row ON row.node_id = ?2 AND row.idx = copy.source
               JOIN {schema}.block_contents AS named ON named.record = row.record
               JOIN {schema}.contents AS source ON source.id = named.content
               JOIN main.contents AS held ON held.hash = source.hash
               JOIN main.blocks AS copied ON copied.node_id = ?1 AND copied.idx = copy.idx"
        ),
        params![node.as_str(), prefix.node.as_str()],
    )?;
    transaction.execute(
        &format!(
            "INSERT INTO block_blobs (node_id, idx, blob)
             SELECT ?1, copy.idx, named.blob
             FROM temp.fork_rows AS copy JOIN {schema}.block_blobs AS named ON named.node_id = ?2 AND named.idx = copy.source"
        ),
        params![node.as_str(), prefix.node.as_str()],
    )?;
    transaction.execute(
        &format!(
            "INSERT INTO block_attachments (node_id, idx, attachment)
             SELECT ?1, copy.idx, named.attachment
             FROM temp.fork_rows AS copy JOIN {schema}.block_attachments AS named ON named.node_id = ?2 AND named.idx = copy.source"
        ),
        params![node.as_str(), prefix.node.as_str()],
    )?;
    transaction.execute(
        "INSERT INTO path_runs (node_id, start, first, length) VALUES (?1, 0, 0, ?2)",
        params![node.as_str(), length],
    )?;
    transaction.execute(
        "UPDATE nodes SET leaf = ?2 WHERE id = ?1",
        params![node.as_str(), length - 1],
    )?;
    transaction.execute_batch("DELETE FROM temp.fork_rows; DELETE FROM temp.fork_dictionaries;")?;
    Ok(Ok(()))
}

/// Starts the path of `node` where `prefix` ends: its first positions name
/// the rows of the node `prefix` names, which stay that node's, and a later
/// switch of that node leaves them as captured. No row is copied, and no
/// block is read.
fn share_prefix(transaction: &Transaction<'_>, node: &NodeId, prefix: &StoredPrefix) -> Result<Result<(), StoreError>, StorageError> {
    match Path::starting_at(transaction, node.as_str(), prefix.node.as_str(), integer(prefix.leaf), count(prefix.length))? {
        Ok(path) => {
            path.write(transaction)?;
            Ok(Ok(()))
        }
        Err(reason) => Ok(Err(StoreError::Failed(reason))),
    }
}

/// The `nodes` row of `record`, with `initial`'s state, and its
/// `side_questions` row for a side question.
fn insert_node(transaction: &Transaction<'_>, record: &NodeRecord, initial: &CheckpointUpdate) -> Result<(), StorageError> {
    let (phase, closed_at, failure) = close_columns(record.closed.as_ref());
    // A side question is no agent: its number is none.
    let number = record.side.is_none().then_some(integer(record.number));
    transaction.execute(
        "INSERT INTO nodes (id, number, parent_id, description, profile, instructions, round, started_at,
           can_spawn, closed_phase, closed_at, failure, delivered, state,
           output_revision, forked_from, cwd, cwd_device, cwd_under, context_limit, started_by, model)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 0, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
        params![
            record.id.as_str(),
            number,
            record.parent.as_ref().map(NodeId::as_str),
            record.description,
            record.profile,
            record.instructions,
            integer(record.round),
            record.started_at.as_millisecond(),
            record.can_spawn_subagents,
            phase,
            closed_at,
            failure,
            record.delivered,
            to_json(&initial.state),
            record.forked_from.map(integer),
            record.cwd.as_ref().map(|own| &own.cwd),
            record.cwd.as_ref().and_then(|own| own.under.device.as_ref()),
            record.cwd.as_ref().map(|own| &own.under.directory),
            record.context_limit,
            record.started_by.as_ref().map(BlockId::as_str),
            record.model.as_ref().map(to_json),
        ],
    )?;
    if let Some(side) = &record.side {
        transaction.execute(
            "INSERT INTO side_questions (node_id, block_id, start_offset, end_offset, quote, picture, question, asked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                record.id.as_str(),
                side.block.as_str(),
                integer(side.start),
                integer(side.end),
                side.quote,
                side.picture.as_ref().map(|picture| serde_json::to_string(picture).expect("a media source serializes")),
                side.question,
                side.asked_at.as_millisecond(),
            ],
        )?;
    }
    Ok(())
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

    fn subagents(&self) -> LocalBoxFuture<'_, Result<Vec<StoredSubagent>, StoreError>> {
        Box::pin(async move {
            let subagents = self.db.call(|connection| subagents(connection)).await.map_err(store_error)?;
            Ok(subagents
                .into_iter()
                .map(|subagent| StoredSubagent {
                    record: subagent.record,
                    started_on_path: subagent.started_on_path,
                })
                .collect())
        })
    }

    fn sides(&self) -> LocalBoxFuture<'_, Result<Vec<NodeRecord>, StoreError>> {
        Box::pin(async move { self.db.call(|connection| sides(connection)).await.map_err(store_error) })
    }

    fn saved_versions(&self) -> LocalBoxFuture<'_, Result<SavedVersions, StoreError>> {
        Box::pin(async move { self.db.call(|connection| saved_versions(connection)).await.map_err(store_error) })
    }

    fn create_node(
        &self,
        record: NodeRecord,
        initial: CheckpointUpdate,
    ) -> LocalBoxFuture<'_, Result<(), StoreError>> {
        Box::pin(self.create(record, Start::Empty, initial))
    }

    fn create_node_after<'a>(
        &'a self,
        record: NodeRecord,
        root: Option<&'a NodeId>,
        prefix: &'a StoredPrefix,
        initial: CheckpointUpdate,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            // Another conversation's rows are read from its file, attached
            // to this one's writer for the copy; this tree's own are named
            // where they are.
            let start = match root {
                Some(root) => {
                    let source = ConversationId::try_from(root.as_str()).map_err(|error| StoreError::Failed(error.to_string()))?;
                    Start::Copied(self.db.file_of(&source).await.map_err(store_error)?, prefix.clone())
                }
                None => Start::Shared(prefix.clone()),
            };
            self.create(record, start, initial).await
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
            let (phase, closed_at, failure) = close_columns(Some(&close));
            let changed = self
                .db
                .call(move |connection| {
                    let changed = connection.execute(
                        "UPDATE nodes SET closed_phase = ?2, closed_at = ?3, failure = ?4, delivered = 0
                         WHERE id = ?1",
                        params![node.as_str(), phase, closed_at, failure],
                    )?;
                    Ok(changed)
                })
                .await
                .map_err(store_error)?;
            if changed == 0 {
                return Err(missing(id));
            }
            (self.saved)(id, None);
            Ok(())
        })
    }

    fn delete_node<'a>(&'a self, id: &'a NodeId) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        let node = id.clone();
        Box::pin(async move {
            // Only a side question's node: a wrong id deletes no other. The
            // node's rows go with it, each table that names it deleting by
            // cascade.
            let changed = self
                .db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    let named: Vec<i64> = transaction
                        .prepare(
                            "SELECT DISTINCT named.content FROM blocks JOIN block_contents AS named ON named.record = blocks.record
                             WHERE blocks.node_id = ?1",
                        )?
                        .query_map([node.as_str()], |row| row.get(0))?
                        .collect::<rusqlite::Result<_>>()?;
                    let changed = transaction.execute(
                        "DELETE FROM nodes WHERE id = ?1 AND id IN (SELECT node_id FROM side_questions)",
                        [node.as_str()],
                    )?;
                    contents::remove_unnamed(&transaction, named)?;
                    transaction.commit()?;
                    Ok(changed)
                })
                .await
                .map_err(store_error)?;
            if changed == 0 {
                return Err(missing(id));
            }
            (self.saved)(id, None);
            Ok(())
        })
    }

    fn reopen_node<'a>(
        &'a self,
        id: &'a NodeId,
        round: u64,
        started_at: Timestamp,
        message: Option<QueuedMessage>,
        reprofile: Option<Reprofile>,
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
                    let state = match message {
                        Some(message) => CheckpointState {
                            queue: vec![message],
                            ..state
                        },
                        None => state,
                    };
                    transaction.execute(
                        "UPDATE nodes SET round = ?2, started_at = ?3, closed_phase = NULL, closed_at = NULL,
                           failure = NULL, delivered = 0, state = ?4
                         WHERE id = ?1",
                        params![node.as_str(), integer(round), started_at.as_millisecond(), to_json(&state)],
                    )?;
                    if let Some(reprofile) = reprofile {
                        transaction.execute(
                            "UPDATE nodes SET profile = ?2, instructions = ?3, context_limit = ?4, can_spawn = ?5, model = ?6
                             WHERE id = ?1",
                            params![
                                node.as_str(),
                                reprofile.name,
                                reprofile.instructions,
                                reprofile.context_limit,
                                reprofile.can_spawn_subagents,
                                to_json(&reprofile.model),
                            ],
                        )?;
                    }
                    transaction.commit()?;
                    Ok(true)
                })
                .await
                .map_err(store_error)?;
            if !found {
                return Err(missing(id));
            }
            (self.saved)(id, None);
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
            (self.saved)(id, None);
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
                    turn: row.turn,
                    title: row.title,
                    script: row.script,
                    service: row.service,
                    started: row.started,
                    unsent: row.unsent,
                })
                .collect())
        })
    }

    fn running_nodes(&self) -> LocalBoxFuture<'_, Result<BTreeSet<NodeId>, StoreError>> {
        Box::pin(async move {
            Ok(self
                .db
                .read(running::nodes)
                .await
                .map_err(store_error)?
                .unwrap_or_default())
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
                        media: row.media,
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
                media: row.media,
            }))
        })
    }
}

impl SessionStore for SqliteSessionStore {
    fn save(&self, update: CheckpointUpdate) -> LocalBoxFuture<'_, Result<(), StoreError>> {
        let node = self.node.clone();
        Box::pin(async move {
            let completions = update.carried_completions()?;
            let change = self
                .db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    let before = node_version_of(&transaction, &node)?;
                    if let Err(refused) =
                        write_checkpoint(&transaction, &node, &update, &completions)?
                    {
                        return Ok(Err(refused));
                    }
                    let change = match (before, node_version_of(&transaction, &node)?) {
                        (Some((agent, before)), Some((_, version))) => {
                            Some(TranscriptChange::new(agent, before, version, &update.changed_blocks))
                        }
                        _ => None,
                    };
                    transaction.commit()?;
                    Ok(Ok(change))
                })
                .await
                .map_err(store_error)??;
            (self.saved)(&self.node, change);
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

    fn blocks(&self, range: Range<usize>) -> LocalBoxFuture<'_, Result<Rows, StoreError>> {
        let node = self.node.clone();
        Box::pin(async move {
            self.db
                .call(move |connection| {
                    let path = Path::read(connection, node.as_str())?;
                    let range = count(range.start)..count(range.end);
                    if range.end > path.len() {
                        return Err(gap(&node, decode("path_runs", "length", usize::try_from(path.len()))?));
                    }
                    Ok(Rows {
                        blocks: path_blocks(connection, &node, &path, range.clone())?,
                        versions: path_versions(connection, &path, range)?,
                    })
                })
                .await
                .map_err(store_error)
        })
    }

    fn position<'a>(&'a self, id: &'a BlockId) -> LocalBoxFuture<'a, Result<Option<usize>, StoreError>> {
        let node = self.node.clone();
        let id = id.clone();
        Box::pin(async move {
            self.db
                .call(move |connection| {
                    let path = Path::read(connection, node.as_str())?;
                    let length = decode("path_runs", "length", usize::try_from(path.len()))?;
                    Transcript::Agent(node).block_index(connection, &id, length)
                })
                .await
                .map_err(store_error)
        })
    }

    fn last_of(&self, kind: Kind, end: usize) -> LocalBoxFuture<'_, Result<Option<usize>, StoreError>> {
        let node = self.node.clone();
        Box::pin(async move {
            self.db
                .call(move |connection| {
                    let path = Path::read(connection, node.as_str())?;
                    last_of(connection, &node, &path, kind, count(end))
                })
                .await
                .map_err(store_error)
        })
    }

    fn next_of(&self, kind: Kind, from: usize) -> LocalBoxFuture<'_, Result<Option<usize>, StoreError>> {
        let node = self.node.clone();
        Box::pin(async move {
            self.db
                .call(move |connection| {
                    let path = Path::read(connection, node.as_str())?;
                    next_of(connection, &node, &path, kind, count(from))
                })
                .await
                .map_err(store_error)
        })
    }

    fn blocks_of(&self, kinds: Vec<Kind>, range: Range<usize>) -> LocalBoxFuture<'_, Result<Vec<(usize, Block)>, StoreError>> {
        let node = self.node.clone();
        Box::pin(async move {
            self.db
                .call(move |connection| {
                    let path = Path::read(connection, node.as_str())?;
                    blocks_of(connection, &node, &path, &kinds, count(range.start)..count(range.end))
                })
                .await
                .map_err(store_error)
        })
    }

    fn turn_position<'a>(&'a self, turn: &'a TurnId) -> LocalBoxFuture<'a, Result<Option<usize>, StoreError>> {
        let node = self.node.clone();
        let turn = turn.clone();
        Box::pin(async move {
            self.db
                .call(move |connection| turn_position(connection, &node, &turn))
                .await
                .map_err(store_error)
        })
    }

    fn earlier(&self, end: usize) -> LocalBoxFuture<'_, Result<EarlierRows, StoreError>> {
        let node = self.node.clone();
        Box::pin(async move {
            self.db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    let path = Path::read(&transaction, node.as_str())?;
                    earlier_rows(&transaction, &node, &path, count(end))
                })
                .await
                .map_err(store_error)
        })
    }

    fn prefix(&self, length: usize) -> LocalBoxFuture<'_, Result<Option<StoredPrefix>, StoreError>> {
        let node = self.node.clone();
        Box::pin(async move {
            self.db
                .call(move |connection| {
                    let path = Path::read(connection, node.as_str())?;
                    if count(length) > path.len() {
                        return Err(gap(&node, length));
                    }
                    let Some(last) = count(length).checked_sub(1).filter(|last| *last >= 0) else {
                        return Ok(None);
                    };
                    let leaf = path.row_at(last).ok_or_else(|| gap(&node, length))?;
                    Ok(Some(StoredPrefix {
                        node: node.clone(),
                        leaf: decode("path_runs", "first", u64::try_from(leaf))?,
                        length,
                    }))
                })
                .await
                .map_err(store_error)
        })
    }

    fn switch(&self, position: usize, version: u32, held_from: usize) -> LocalBoxFuture<'_, Result<Switched, StoreError>> {
        let node = self.node.clone();
        Box::pin(async move {
            let switched = self
                .db
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    let before = node_version_of(&transaction, &node)?;
                    let mut path = Path::read(&transaction, node.as_str())?;
                    let Some(at) = path.switch(&transaction, count(position), version)? else {
                        return Ok(Err(StoreError::Failed(format!(
                            "position {position} of node {node} holds no version {version}"
                        ))));
                    };
                    let from = decode("path_runs", "start", usize::try_from(at))?;
                    path.write(&transaction)?;
                    // A change of the path is output.
                    transaction.execute(
                        "UPDATE nodes SET cut = NULL, output_revision = output_revision + 1 WHERE id = ?1",
                        [node.as_str()],
                    )?;
                    let blocks = path_blocks(&transaction, &node, &path, at..path.len())?;
                    let versions = path
                        .versions(&transaction, at..path.len())?
                        .into_iter()
                        .map(|(position, versions)| {
                            Ok((decode("path_runs", "start", usize::try_from(position))?, versions))
                        })
                        .collect::<Result<_, StorageError>>()?;
                    // The search index holds the agent's path: the blocks
                    // from the switched position replace its rows from there.
                    let change = match (before, node_version_of(&transaction, &node)?) {
                        (Some((agent, before)), Some((_, version))) => {
                            let written: Vec<(usize, Block)> = (from..).zip(blocks.iter().cloned()).collect();
                            Some(TranscriptChange::new(agent, before, version, &written))
                        }
                        _ => None,
                    };
                    // What the caller's window lacks before the place is
                    // read in the same transaction, so nothing after the
                    // commit can fail.
                    let before = if from < held_from {
                        let start = last_of(&transaction, &node, &path, Kind::CompactionBoundary, at + 1)?.unwrap_or(0);
                        let range = count(start)..at;
                        Some(SwitchedBefore {
                            start,
                            rows: Rows {
                                blocks: path_blocks(&transaction, &node, &path, range.clone())?,
                                versions: path_versions(&transaction, &path, range)?,
                            },
                            earlier: earlier_rows(&transaction, &node, &path, count(start))?,
                        })
                    } else {
                        None
                    };
                    transaction.commit()?;
                    Ok(Ok((Switched { from, blocks, versions, before }, change)))
                })
                .await
                .map_err(store_error)??;
            let (switched, change) = switched;
            (self.saved)(&self.node, change);
            Ok(switched)
        })
    }

    fn blobs(&self) -> &dyn BlobStore {
        &*self.blobs
    }
}

/// Writes one save of `node` in `transaction`.
///
/// - From the cut, the rows the path held leave it and stay as they are, and each
///   block from there is a new row following the one before it.
/// - Below the cut, a changed block updates its row.
/// - The rows carry the blobs and attachments each names, and the state row and
///   the path's runs go with them.
/// - The child completions it carries are marked delivered.
///
/// Changed output, or a path cut, advances the node's output revision; input
/// alone does not. A refusal leaves the transaction uncommitted.
fn write_checkpoint(
    transaction: &Transaction<'_>,
    node: &NodeId,
    update: &CheckpointUpdate,
    completions: &[CompletionId],
) -> Result<Result<(), StoreError>, StorageError> {
    let stored_cut: Option<Option<i64>> = transaction
        .query_row("SELECT cut FROM nodes WHERE id = ?1", [node.as_str()], |row| row.get(0))
        .optional()?;
    let Some(mut replaced) = stored_cut else {
        return Ok(Err(missing(node)));
    };
    let mut path = Path::read(transaction, node.as_str())?;
    let stored = path.len();
    let cut = update
        .moved_from
        .map_or(i64::MAX, count)
        .min(count(update.block_count))
        .min(stored);
    // The rows the node starts from are settled and another node's.
    if cut < path.base() {
        return Ok(Err(StoreError::Failed(format!(
            "a save of node {node} cuts its path at position {cut}, inside the rows it starts from"
        ))));
    }
    // A block the save carries to the new branch under its id, as a
    // compaction inside a turn does, replaces its row there, whose
    // versions it keeps.
    let mut moved = HashMap::new();
    if cut < stored {
        replaced = path.row_at(cut);
        let mut ids = transaction.prepare_cached(
            "SELECT idx, id FROM blocks WHERE node_id = ?1 AND idx >= ?2 AND idx < ?3",
        )?;
        for segment in path.segments(node.as_str(), cut..stored) {
            let found = ids
                .query_map(params![segment.node, segment.rows.start, segment.rows.end], |row| {
                    Ok((row.get::<_, String>(1)?, row.get::<_, i64>(0)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            moved.extend(found);
        }
        path.truncate(cut);
    }
    // An edit's or a Regenerate's next row is a version of its own.
    if update.version {
        replaced = None;
    }
    let mut next = path::next_row(transaction, node.as_str())?;
    let codec = Codec::new(transaction);
    for (position, block) in &update.changed_blocks {
        let position = count(*position);
        if position < cut {
            let Some(row) = path.row_at(position) else {
                return Ok(Err(StoreError::Failed(format!(
                    "a save of node {node} rewrites position {position}, which is in the rows it starts from"
                ))));
            };
            shares::keep_rewritten(transaction, &codec, node, row)?;
            records::update(transaction, &codec, node.as_str(), row, block)?;
            name_block(transaction, node.as_str(), row, block)?;
            continue;
        }
        if position != path.len() {
            return Ok(Err(StoreError::Failed(format!(
                "a save of node {node} skips position {}",
                path.len()
            ))));
        }
        let replaces = match moved.remove(block.id().as_str()) {
            Some(row) => {
                replaced = None;
                Some(row)
            }
            None => replaced.take(),
        };
        records::insert(transaction, &codec, node.as_str(), next, path.leaf(), replaces, block)?;
        name_block(transaction, node.as_str(), next, block)?;
        path.push(next);
        next += 1;
    }
    if path.len() != count(update.block_count) {
        return Ok(Err(StoreError::Failed(format!(
            "a save of node {node} leaves {} of its {} blocks on its path",
            path.len(),
            update.block_count
        ))));
    }
    drop(codec);
    records::train_when_due(transaction)?;
    path.write(transaction)?;
    let output = cut < stored
        || update
            .changed_blocks
            .iter()
            .any(|(_, block)| is_output(block));
    transaction.execute(
        "UPDATE nodes SET state = ?2, cut = ?3, output_revision = output_revision + ?4 WHERE id = ?1",
        params![node.as_str(), to_json(&update.state), replaced, i64::from(output)],
    )?;
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
    if let Block::Text(text) = block {
        let named = shares::block_attachments(text);
        if !named.is_empty() {
            let mut attachment = transaction.prepare_cached(
                "INSERT OR IGNORE INTO block_attachments (node_id, idx, attachment) VALUES (?1, ?2, ?3)",
            )?;
            for number in named {
                let number = i64::try_from(number.0).expect("a parsed attachment number fits the column");
                attachment.execute(params![node, index, number])?;
            }
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

/// A node's checkpoint: its state row and its window, the blocks of its
/// current path from the last `compaction_boundary` on with the versions
/// there; none when the node does not exist. No row before the window is
/// read.
fn checkpoint(connection: &Connection, node: &NodeId) -> Result<Option<Checkpoint>, StorageError> {
    let row: Option<String> = connection
        .query_row("SELECT state FROM nodes WHERE id = ?1", [node.as_str()], |row| row.get(0))
        .optional()?;
    let Some(state) = row else {
        return Ok(None);
    };
    let path = Path::read(connection, node.as_str())?;
    let start = last_of(connection, node, &path, Kind::CompactionBoundary, path.len())?.unwrap_or(0);
    let window = count(start)..path.len();
    Ok(Some(Checkpoint {
        state: json("nodes", "state", &state)?,
        start,
        transcript: path_blocks(connection, node, &path, window.clone())?,
        versions: path_versions(connection, &path, window)?,
    }))
}

/// The versions at the positions `range` of `path`, by position.
fn path_versions(
    connection: &Connection,
    path: &Path,
    range: Range<i64>,
) -> Result<BTreeMap<usize, demi_conversation_lane_protocol::Versions>, StorageError> {
    path.versions(connection, range)?
        .into_iter()
        .map(|(position, versions)| Ok((decode("path_runs", "start", usize::try_from(position))?, versions)))
        .collect()
}

/// What a block of one of `kind`'s types must also hold, as an SQL condition
/// on the columns of its row.
fn condition(kind: Kind) -> &'static str {
    match kind {
        Kind::ExecutingCall => "status = 'executing'",
        Kind::CompletedText => "forkable = 1",
        Kind::Record => "outside_turn = 0 AND (type != 'wakeup' OR placement != 'steer')",
        _ => "1",
    }
}

/// The position of the last block of `kind` on `node`'s `path` before
/// `end`, found through the index of block types from the path's last run
/// back, so no block is decoded.
fn last_of(
    connection: &Connection,
    node: &NodeId,
    path: &Path,
    kind: Kind,
    end: i64,
) -> Result<Option<usize>, StorageError> {
    let types = serde_json::to_string(kind.types()).expect("type names serialize");
    let mut statement = connection.prepare_cached(&format!(
        "SELECT max(idx) FROM blocks INDEXED BY blocks_by_type
         WHERE node_id = ?1 AND type IN (SELECT value FROM json_each(?2))
           AND idx >= ?3 AND idx < ?4 AND ({})",
        condition(kind)
    ))?;
    for Segment { start, node: holder, rows } in path.segments(node.as_str(), 0..end.min(path.len())).into_iter().rev() {
        let found: Option<i64> =
            statement.query_row(params![holder, types, rows.start, rows.end], |row| row.get(0))?;
        if let Some(row) = found {
            return Ok(Some(decode("path_runs", "start", usize::try_from(start + row - rows.start))?));
        }
    }
    Ok(None)
}

/// The position of the first block of `kind` on `node`'s `path` at or
/// after `from`, found through the index of block types from that run on.
fn next_of(
    connection: &Connection,
    node: &NodeId,
    path: &Path,
    kind: Kind,
    from: i64,
) -> Result<Option<usize>, StorageError> {
    let types = serde_json::to_string(kind.types()).expect("type names serialize");
    let mut statement = connection.prepare_cached(&format!(
        "SELECT min(idx) FROM blocks INDEXED BY blocks_by_type
         WHERE node_id = ?1 AND type IN (SELECT value FROM json_each(?2))
           AND idx >= ?3 AND idx < ?4 AND ({})",
        condition(kind)
    ))?;
    for Segment { start, node: holder, rows } in path.segments(node.as_str(), from..path.len()) {
        let found: Option<i64> =
            statement.query_row(params![holder, types, rows.start, rows.end], |row| row.get(0))?;
        if let Some(row) = found {
            return Ok(Some(decode("path_runs", "start", usize::try_from(start + row - rows.start))?));
        }
    }
    Ok(None)
}

/// The blocks of `kinds` at the positions `range` of `node`'s `path`, each
/// with its position, found through the index of block types: only they
/// are read and decoded.
fn blocks_of(
    connection: &Connection,
    node: &NodeId,
    path: &Path,
    kinds: &[Kind],
    range: Range<i64>,
) -> Result<Vec<(usize, Block)>, StorageError> {
    let types: Vec<&str> = kinds.iter().flat_map(|kind| kind.types().iter().copied()).collect();
    let types = serde_json::to_string(&types).expect("type names serialize");
    let mut statement = connection.prepare_cached(
        "SELECT idx, format, dictionary, body FROM blocks INDEXED BY blocks_by_type
         WHERE node_id = ?1 AND type IN (SELECT value FROM json_each(?2))
           AND idx >= ?3 AND idx < ?4
         ORDER BY idx",
    )?;
    let codec = Codec::new(connection);
    let mut found = Vec::new();
    for Segment { start, node: holder, rows } in path.segments(node.as_str(), range.start..range.end.min(path.len())) {
        let mut read = statement.query(params![holder, types, rows.start, rows.end])?;
        while let Some(row) = read.next()? {
            let idx: i64 = row.get(0)?;
            let block = codec.block("blocks", &Stored::of(row, 1)?)?;
            if kinds.iter().any(|kind| kind.holds(&block)) {
                found.push((decode("path_runs", "start", usize::try_from(start + idx - rows.start))?, block));
            }
        }
    }
    Ok(found)
}

/// The position of the `user` block that opened `turn` on `node`'s
/// current path, found through the index of block types.
fn turn_position(connection: &Connection, node: &NodeId, turn: &TurnId) -> Result<Option<usize>, StorageError> {
    let position: Option<i64> = connection
        .prepare_cached(&format!(
            "SELECT min({POSITION}) FROM path_runs AS run CROSS JOIN blocks INDEXED BY blocks_by_type
               ON blocks.node_id = {RUN_NODE} AND blocks.type = 'user'
                 AND blocks.idx >= run.first AND blocks.idx < run.first + run.length
             WHERE run.node_id = ?1 AND blocks.turn_id = ?2"
        ))?
        .query_row(params![node.as_str(), turn.as_str()], |row| row.get(0))?;
    position.map(|position| decode("path_runs", "start", usize::try_from(position))).transpose()
}

/// What a session keeps of the blocks of `node`'s `path` before `end`
/// ([`SessionStore::earlier`]).
fn earlier_rows(connection: &Connection, node: &NodeId, path: &Path, end: i64) -> Result<EarlierRows, StorageError> {
    let ids = earlier_ids(connection, node, end)?;
    let last = |kind: Kind| -> Result<Option<Block>, StorageError> {
        match last_of(connection, node, path, kind, end)? {
            Some(at) => Ok(path_blocks(connection, node, path, count(at)..count(at) + 1)?.pop()),
            None => Ok(None),
        }
    };
    Ok(EarlierRows {
        user_turns: ids.0,
        calls: ids.1,
        last_input: last(Kind::Input)?,
        last_user_input: last(Kind::UserInput)?,
        last_record: last(Kind::Record)?,
    })
}

/// The turns of the `user` blocks and the ids of the tool calls on
/// `node`'s current path before `end`, read from the index of block types
/// without decoding a block.
fn earlier_ids(connection: &Connection, node: &NodeId, end: i64) -> Result<(Vec<TurnId>, Vec<BlockId>), StorageError> {
    let mut statement = connection.prepare_cached(&format!(
        "SELECT blocks.type, CASE blocks.type WHEN 'user' THEN blocks.turn_id ELSE blocks.id END
         FROM path_runs AS run CROSS JOIN blocks INDEXED BY blocks_by_type
           ON blocks.node_id = {RUN_NODE} AND blocks.type IN ('user', 'tool_call')
             AND blocks.idx >= run.first AND blocks.idx < run.first + run.length
         WHERE run.node_id = ?1 AND {POSITION} < ?2
         ORDER BY run.start, blocks.idx"
    ))?;
    let mut rows = statement.query(params![node.as_str(), end])?;
    let mut ids = (Vec::new(), Vec::new());
    while let Some(row) = rows.next()? {
        let kind: String = row.get(0)?;
        let id: String = row.get(1)?;
        if kind == "user" {
            ids.0.push(decode("blocks", "block", TurnId::try_from(id))?);
        } else {
            ids.1.push(decode("blocks", "block", BlockId::try_from(id))?);
        }
    }
    Ok(ids)
}

/// The blocks at the positions `range` of `node`'s path, which it holds.
pub(crate) fn path_blocks(
    connection: &Connection,
    node: &NodeId,
    path: &Path,
    range: Range<i64>,
) -> Result<Vec<Block>, StorageError> {
    let mut blocks = Vec::new();
    each_block(connection, node, path, range, |_, block| {
        blocks.push(block);
        ControlFlow::Continue(())
    })?;
    Ok(blocks)
}

/// The blocks at the positions `range` of `node`'s current path, all of
/// which it holds.
pub fn agent_blocks(connection: &Connection, node: &NodeId, range: Range<usize>) -> Result<Vec<Block>, StorageError> {
    let path = Path::read(connection, node.as_str())?;
    path_blocks(connection, node, &path, count(range.start)..count(range.end))
}

/// Hands each block at the positions `range` of `node`'s path to `take` in
/// order, with its position, decoded one at a time: exactly one row for each
/// position of the range, until `take` breaks, after which no row is read.
fn each_block(
    connection: &Connection,
    node: &NodeId,
    path: &Path,
    range: Range<i64>,
    mut take: impl FnMut(usize, Block) -> ControlFlow<()>,
) -> Result<(), StorageError> {
    let mut statement = connection
        .prepare_cached("SELECT idx, format, dictionary, body FROM blocks WHERE node_id = ?1 AND idx >= ?2 AND idx < ?3 ORDER BY idx")?;
    let codec = Codec::new(connection);
    let first = decode("path_runs", "start", usize::try_from(range.start))?;
    let mut position = first;
    for Segment { node: holder, rows, .. } in path.segments(node.as_str(), range.clone()) {
        let mut found = statement.query(params![holder, rows.start, rows.end])?;
        let mut expected = rows.start;
        while let Some(row) = found.next()? {
            if row.get::<_, i64>(0)? != expected {
                return Err(gap(node, position));
            }
            if take(position, codec.block("blocks", &Stored::of(row, 1)?)?).is_break() {
                return Ok(());
            }
            position += 1;
            expected += 1;
        }
        if expected != rows.end {
            return Err(gap(node, position));
        }
    }
    if count(position - first) != range.end - range.start {
        return Err(gap(node, position));
    }
    Ok(())
}

fn gap(node: &NodeId, index: usize) -> StorageError {
    StorageError::Corrupt {
        table: "blocks",
        column: "idx",
        reason: format!("node {node} has no block row at position {index} of its path"),
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

/// The columns of a node's record, from [`NODES`].
const NODE_COLUMNS: &str =
    "id, number, parent_id, description, profile, instructions, round, started_at, can_spawn,
     closed_phase, closed_at, failure, delivered, forked_from, cwd, cwd_device, cwd_under,
     context_limit, model, started_by, side_questions.block_id AS side_block, start_offset, end_offset, quote, picture,
     question, asked_at";

/// The nodes with what a side question's node asks about.
const NODES: &str = "nodes LEFT JOIN side_questions ON side_questions.node_id = nodes.id";

fn node_by_id(connection: &Connection, id: &NodeId) -> Result<Option<NodeRecord>, StorageError> {
    let mut statement =
        connection.prepare(&format!("SELECT {NODE_COLUMNS} FROM {NODES} WHERE id = ?1"))?;
    let mut rows = statement.query([id.as_str()])?;
    rows.next()?.map(node_row).transpose()
}

/// A node's direct children in spawn order, the order of their numbers; a
/// side question is none.
fn children_of(connection: &Connection, parent: &NodeId) -> Result<Vec<NodeRecord>, StorageError> {
    let mut statement = connection.prepare(&format!(
        "SELECT {NODE_COLUMNS} FROM {NODES} WHERE parent_id = ?1 AND number IS NOT NULL ORDER BY number"
    ))?;
    let mut rows = statement.query([parent.as_str()])?;
    let mut children = Vec::new();
    while let Some(row) = rows.next()? {
        children.push(node_row(row)?);
    }
    Ok(children)
}

/// The versions of the tree's stored state: each node's transcript at its
/// output revision with its path's length, and the agents at a hash of the rows
/// that decide their entries. A save that changes what a page shows of either
/// changes its version, so a page compares what it read with what the store
/// holds now; a tree with no row has the empty one.
pub fn saved_versions(connection: &Connection) -> Result<SavedVersions, StorageError> {
    // Revisions and lengths stay below these bounds, so the pair is one
    // number a page holds exactly.
    const LENGTH_BITS: u32 = 26;
    const SAFE: u64 = (1 << 53) - 1;
    let mut statement = connection.prepare(&format!(
        "SELECT id, output_revision, {PATH_LENGTH}, round, closed_phase, closed_at, delivered, number \
         FROM nodes ORDER BY id"
    ))?;
    let mut rows = statement.query([])?;
    let mut nodes = BTreeMap::new();
    // FNV-1a over the rows' columns.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes.iter().chain(&[0xff]) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let revision: i64 = row.get(1)?;
        let length: i64 = row.get(2)?;
        feed(id.as_bytes());
        feed(&revision.to_le_bytes());
        feed(&length.to_le_bytes());
        feed(&row.get::<_, i64>(3)?.to_le_bytes());
        feed(row.get::<_, Option<String>>(4)?.unwrap_or_default().as_bytes());
        feed(&row.get::<_, Option<i64>>(5)?.unwrap_or(-1).to_le_bytes());
        feed(&row.get::<_, i64>(6)?.to_le_bytes());
        feed(&row.get::<_, Option<i64>>(7)?.unwrap_or(-1).to_le_bytes());
        let version = (u64::try_from(revision).unwrap_or(0) << LENGTH_BITS) | u64::try_from(length).unwrap_or(0);
        nodes.insert(decode("nodes", "id", NodeId::try_from(id))?, version & SAFE);
    }
    Ok(SavedVersions { agents: hash & SAFE, nodes })
}

/// The tree's side questions, in the order they were asked.
fn sides(connection: &Connection) -> Result<Vec<NodeRecord>, StorageError> {
    let mut statement = connection.prepare(&format!(
        "SELECT {NODE_COLUMNS} FROM {NODES} WHERE side_questions.node_id IS NOT NULL ORDER BY asked_at, id"
    ))?;
    let mut rows = statement.query([])?;
    let mut sides = Vec::new();
    while let Some(row) = rows.next()? {
        sides.push(node_row(row)?);
    }
    Ok(sides)
}

/// What a side question's node asks about, from its `side_questions` row;
/// none for an agent, which has no number only if its row is missing.
fn side_of(row: &Row<'_>) -> Result<Option<SideOf>, StorageError> {
    const TABLE: &str = "side_questions";
    let Some(block) = row.get::<_, Option<String>>("side_block")? else {
        return Ok(None);
    };
    Ok(Some(SideOf {
        block: decode(TABLE, "block_id", BlockId::try_from(block))?,
        start: decode(TABLE, "start_offset", u64::try_from(row.get::<_, i64>("start_offset")?))?,
        end: decode(TABLE, "end_offset", u64::try_from(row.get::<_, i64>("end_offset")?))?,
        quote: row.get("quote")?,
        picture: super::side_questions::picture(row, TABLE)?,
        question: row.get("question")?,
        asked_at: instant(row, TABLE, "asked_at")?,
    }))
}

/// A `nodes` row with its side question's, read from its columns in
/// `NODE_COLUMNS`.
fn node_row(row: &Row<'_>) -> Result<NodeRecord, StorageError> {
    const TABLE: &str = "nodes";
    let parent: Option<String> = row.get("parent_id")?;
    let side = side_of(row)?;
    // A side question takes no number, and every agent has one.
    let number = match (row.get::<_, Option<i64>>("number")?, &side) {
        (Some(number), None) => decode(TABLE, "number", u64::try_from(number))?,
        (None, Some(_)) => 0,
        _ => {
            return Err(StorageError::Corrupt {
                table: TABLE,
                column: "number",
                reason: "a node is an agent with a number or a side question without one".into(),
            });
        }
    };
    let phase: Option<String> = row.get("closed_phase")?;
    let closed = match phase {
        None => None,
        Some(phase) => {
            let phase = match phase.as_str() {
                "completed" => ClosePhase::Completed,
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
    // A child runs on a model of its own; the root's the conversation's record
    // holds, and a side question's its checkpoint does.
    let model = match row.get::<_, Option<String>>("model")? {
        Some(model) => Some(json::<ModelSelection>(TABLE, "model", &model)?),
        None => None,
    };
    if model.is_some() != (parent.is_some() && side.is_none()) {
        return Err(StorageError::Corrupt {
            table: TABLE,
            column: "model",
            reason: "a child has a model, and the root and a side question none".into(),
        });
    }
    // A side question works in the conversation's directory, as the root.
    if cwd.is_some() != (parent.is_some() && side.is_none()) {
        return Err(StorageError::Corrupt {
            table: TABLE,
            column: "cwd",
            reason: "a child has a working directory, and the root and a side question none".into(),
        });
    }
    Ok(NodeRecord {
        id: decode(TABLE, "id", NodeId::try_from(row.get::<_, String>("id")?))?,
        number,
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
        model,
        round: decode(TABLE, "round", u64::try_from(row.get::<_, i64>("round")?))?,
        started_at: instant(row, TABLE, "started_at")?,
        can_spawn_subagents: row.get("can_spawn")?,
        closed,
        delivered: row.get("delivered")?,
        forked_from: row
            .get::<_, Option<i64>>("forked_from")?
            .map(|number| decode(TABLE, "forked_from", u64::try_from(number)))
            .transpose()?,
        started_by: row
            .get::<_, Option<String>>("started_by")?
            .map(|block| decode(TABLE, "started_by", BlockId::try_from(block)))
            .transpose()?,
        cwd,
        side,
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

/// A close as its columns: the phase, the time, and the failure a failed
/// phase carries.
fn close_columns(close: Option<&NodeClose>) -> (Option<&'static str>, Option<i64>, Option<String>) {
    let Some(close) = close else {
        return (None, None, None);
    };
    let at = Some(close.at.as_millisecond());
    match &close.phase {
        ClosePhase::Completed => (Some("completed"), at, None),
        ClosePhase::Aborted => (Some("aborted"), at, None),
        ClosePhase::Error { failure } => (Some("error"), at, Some(failure.clone())),
    }
}

/// A count as the INTEGER column holds it: an agent's number counts spawns,
/// a round resumes and a revision saves, so none comes near `i64::MAX`.
fn integer(value: u64) -> i64 {
    i64::try_from(value).expect("a count fits the column")
}

/// What a conversation's summary is built from: the root's phase and output
/// revision, the kind of its latest terminal block and its latest ended
/// turn, read without loading the transcript, the speed its checkpoint
/// keeps, and the number of its latest attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryFacts {
    pub phase: SessionPhase,
    pub revision: u64,
    pub last: Option<Terminal>,
    pub last_turn: Option<EndedTurn>,
    pub turn_speed: Option<TurnSpeed>,
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
        turn_speed: None,
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

/// The root's latest ended turn.
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

/// The root's output revision, 0 for a conversation that has no tree yet: what
/// a read acknowledgement is checked against, read alone.
pub fn output_revision(connection: &Connection) -> Result<u64, StorageError> {
    let revision: Option<i64> = connection
        .prepare_cached("SELECT output_revision FROM nodes WHERE parent_id IS NULL")?
        .query_row([], |row| row.get(0))
        .optional()?;
    revision
        .map(|revision| decode("nodes", "output_revision", u64::try_from(revision)))
        .transpose()
        .map(Option::unwrap_or_default)
}

/// The summary facts of the tree `connection` holds; the empty facts when it
/// has no root yet.
pub fn summary(connection: &Connection) -> Result<SummaryFacts, StorageError> {
    let root: Option<(String, String, i64)> = connection
        .query_row(
            "SELECT id, state, output_revision FROM nodes WHERE parent_id IS NULL",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let last_attachment = conversation_attachments::last(connection)?;
    let Some((id, state, revision)) = root else {
        return Ok(SummaryFacts {
            last_attachment,
            ..SummaryFacts::EMPTY
        });
    };
    let state: CheckpointState = json("nodes", "state", &state)?;
    // A failed `compact` ends no turn, and a stop that a Resume continued
    // ended the turn only until the Resume.
    let mut rows = connection.prepare_cached(&format!(
        "SELECT blocks.idx, blocks.type, blocks.turn_id,
                blocks.type IN ('response', 'error', 'abort') AND blocks.outside_turn = 0 AND blocks.resumed = 0,
                blocks.format, blocks.dictionary, CASE blocks.type WHEN 'text' THEN blocks.body END
           FROM {PATH_ROWS} WHERE run.node_id = ?1 ORDER BY run.start DESC, blocks.idx DESC"
    ))?;
    let codec = Codec::new(connection);
    // A text is joined only when the walk reaches it.
    let rows = rows.query_map(params![id], |row| {
        let body: Option<Vec<u8>> = row.get(6)?;
        Ok((
            WalkRow {
                index: row.get(0)?,
                kind: row.get(1)?,
                turn: row.get(2)?,
                ends_turn: row.get(3)?,
                text: None,
            },
            match body {
                Some(body) => Some(Stored {
                    format: row.get(4)?,
                    dictionary: row.get(5)?,
                    body,
                }),
                None => None,
            },
        ))
    })?;
    let rows = rows.map(|row| {
        let (mut walked, stored) = row?;
        if let Some(stored) = stored {
            walked.text = match codec.block("blocks", &stored)? {
                Block::Text(text) => Some(text.text),
                _ => None,
            };
        }
        Ok(walked)
    });
    let walk = walk(rows, state.phase)?;
    let last_turn = match walk.end {
        Some(end) => Some(ended_turn(connection, &id, end, walk.answer)?),
        None => None,
    };
    Ok(SummaryFacts {
        phase: state.phase,
        revision: decode("nodes", "output_revision", u64::try_from(revision))?,
        last: walk.last,
        last_turn,
        turn_speed: state.turn_speed,
        last_attachment,
    })
}

/// Walks the root's block rows back from its last one, until it knows the latest
/// terminal block and the latest ended turn with its last text.
///
/// A turn's input blocks carry its id, and the turn lasts until a block of another
/// turn's input, so a text belongs to the ended turn when it comes after one of
/// that turn's input blocks.
///
/// While the root is in a turn (`phase` is not idle) that turn has not ended, so
/// what the walk read of it, such as the responses of its earlier requests, counts
/// for nothing. An action that has written no block of its own yet, such as a
/// `compact` or a send before its message is written, cannot be told from the turn
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
    let stored = connection.query_row(
        "SELECT format, dictionary, body FROM blocks WHERE node_id = ?1 AND idx = ?2",
        params![node, end],
        |row| Stored::of(row, 0),
    )?;
    let (id, outcome) = match Codec::new(connection).block("blocks", &stored)? {
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

/// Which saved state of the root's transcript a reader saw: its output revision
/// and its block count.
///
/// Every save that changes the transcript changes one of them: a save of output,
/// or one that removes rows, advances the revision, and a save of input alone adds
/// blocks. The transcript's own version, its epoch and revision, lives only in the
/// running session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SavedVersion {
    pub revision: u64,
    pub blocks: u64,
}

/// The node's number, 0 for the root, and the saved version of its
/// transcript, its current path's length as its block count; none when the
/// tree has no such agent, as for a side question, which the search index
/// leaves out.
fn node_version_of(transaction: &Transaction<'_>, node: &NodeId) -> Result<Option<(u64, SavedVersion)>, StorageError> {
    let row: Option<(i64, i64, i64)> = transaction
        .prepare_cached(&format!(
            "SELECT number, output_revision, {PATH_LENGTH} FROM nodes WHERE id = ?1 AND number IS NOT NULL"
        ))?
        .query_row([node.as_str()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .optional()?;
    row.map(|(number, revision, blocks)| {
        Ok((
            decode("nodes", "number", u64::try_from(number))?,
            SavedVersion {
                revision: decode("nodes", "output_revision", u64::try_from(revision))?,
                blocks: decode("path_runs", "length", u64::try_from(blocks))?,
            },
        ))
    })
    .transpose()
}

/// The node's number, 0 for the root: the agent the search index keys its
/// rows by; none when the tree has no such agent, as for a side question.
pub fn agent_number(connection: &Connection, node: &NodeId) -> Result<Option<u64>, StorageError> {
    let number: Option<i64> = connection
        .query_row("SELECT number FROM nodes WHERE id = ?1 AND number IS NOT NULL", [node.as_str()], |row| row.get(0))
        .optional()?;
    number.map(|number| decode("nodes", "number", u64::try_from(number))).transpose()
}

/// The agent numbered `number`, 0 for the root; none when the tree has no
/// such agent, as for a side question, which has no number.
pub fn node_numbered(connection: &Connection, number: u64) -> Result<Option<NodeId>, StorageError> {
    let Ok(number) = i64::try_from(number) else {
        return Ok(None);
    };
    let id: Option<String> = connection
        .query_row("SELECT id FROM nodes WHERE number = ?1", [number], |row| row.get(0))
        .optional()?;
    id.map(|id| decode("nodes", "id", NodeId::try_from(id))).transpose()
}

/// The saved version of each agent's transcript, by the agent's number,
/// which the search index compares with what it holds.
pub fn search_versions(connection: &Connection) -> Result<BTreeMap<u64, SavedVersion>, StorageError> {
    Ok(saved_nodes(connection)?
        .into_iter()
        .map(|(_, agent, version)| (agent, version))
        .collect())
}

/// Each agent's id, number and the saved version of its transcript; a side
/// question is no agent, and nobody searches it.
fn saved_nodes(connection: &Connection) -> Result<Vec<(NodeId, u64, SavedVersion)>, StorageError> {
    let mut statement = connection.prepare(&format!(
        "SELECT id, number, output_revision, {PATH_LENGTH} FROM nodes WHERE number IS NOT NULL"
    ))?;
    let mut rows = statement.query([])?;
    let mut nodes = Vec::new();
    while let Some(row) = rows.next()? {
        nodes.push((
            decode("nodes", "id", NodeId::try_from(row.get::<_, String>(0)?))?,
            decode("nodes", "number", u64::try_from(row.get::<_, i64>(1)?))?,
            SavedVersion {
                revision: decode("nodes", "output_revision", u64::try_from(row.get::<_, i64>(2)?))?,
                blocks: decode("path_runs", "length", u64::try_from(row.get::<_, i64>(3)?))?,
            },
        ));
    }
    Ok(nodes)
}

/// The searchable text of every agent's current path with the saved
/// version it makes, read together; each block is decoded and dropped in
/// turn, so no transcript is held whole.
pub fn search_texts(connection: &Connection) -> Result<Vec<IndexedAgent>, StorageError> {
    let mut agents = Vec::new();
    for (node, agent, version) in saved_nodes(connection)? {
        let path = Path::read(connection, node.as_str())?;
        let mut messages = Vec::new();
        each_block(connection, &node, &path, 0..path.len(), |position, block| {
            if let Some(text) = searchable_text(&block) {
                messages.push(IndexedMessage {
                    block: block.id().clone(),
                    position,
                    text,
                });
            }
            ControlFlow::Continue(())
        })?;
        agents.push(IndexedAgent { agent, version, messages });
    }
    Ok(agents)
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

/// A transcript a page is read from: an agent's in the tree, or what a share's list
/// shows of one. The pages, their light form and a block read whole are the same for
/// both; only the rows they come from differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transcript {
    /// The transcript of the node, the blocks below its block count.
    Agent(NodeId),
    /// The share's list of the token: each entry at its position on the
    /// list, a block read from the row the entry names or from the share's
    /// copy, or a run of steps not shared. The list is the whole of what it
    /// reads: no read reaches a row the list does not name.
    Shared { token: ShareToken },
}

/// What a transcript holds at an index: a block, or, on a share's list only, the number
/// of steps not shared in a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Block(Block),
    StepsNotShared(u64),
}

impl Entry {
    /// Its block; none for a run of steps.
    pub fn block(&self) -> Option<&Block> {
        match self {
            Self::Block(block) => Some(block),
            Self::StepsNotShared(_) => None,
        }
    }

    /// The id of its block; none for a run of steps.
    pub fn id(&self) -> Option<&BlockId> {
        self.block().map(Block::id)
    }
}

/// The entries of a share's list, as the `FROM` of a query that names them
/// `entry`, `row` and `copy`: each entry with the block it shows, its row's
/// or its copy's, or neither for a run of steps. `?1` is the token.
const LISTED: &str = "share_entries AS entry
  LEFT JOIN blocks AS row ON row.node_id = entry.node_id AND row.idx = entry.idx
  LEFT JOIN share_blocks AS copy
    ON entry.node_id IS NULL AND copy.token = entry.token AND copy.idx = entry.position";

/// The columns of the record an entry of [`LISTED`] shows, its row's or its
/// copy's, which [`listed`] reads.
fn listed_record() -> String {
    format!("{}, {}", record_of("row"), record_of("copy"))
}

/// The record an entry of [`LISTED`] shows, from the columns
/// [`listed_record`] selected at `first`; none for an entry whose row or
/// copy is gone, or a run of steps.
fn listed(row: &Row<'_>, first: usize) -> rusqlite::Result<Option<Stored>> {
    let own: Option<Vec<u8>> = row.get(first + 2)?;
    if own.is_some() {
        return Stored::of(row, first).map(Some);
    }
    let copy: Option<Vec<u8>> = row.get(first + 5)?;
    copy.map(|_| Stored::of(row, first + 3)).transpose()
}

impl Transcript {
    /// How many entries the transcript holds; none when there is no such
    /// transcript: a node the tree lacks. A share always holds its list.
    pub fn length(&self, connection: &Connection) -> Result<Option<usize>, StorageError> {
        let length: Option<i64> = match self {
            Self::Agent(node) => connection
                .query_row(
                    &format!("SELECT {PATH_LENGTH} FROM nodes WHERE id = ?1"),
                    [node.as_str()],
                    |row| row.get(0),
                )
                .optional()?,
            Self::Shared { token } => Some(connection.query_row(
                "SELECT COUNT(*) FROM share_entries WHERE token = ?1",
                [token.as_str()],
                |row| row.get(0),
            )?),
        };
        length
            .map(|length| decode("path_runs", "length", usize::try_from(length)))
            .transpose()
    }

    /// Its entries at the indices `range`, all of which it holds.
    pub fn entries_in(&self, connection: &Connection, range: Range<usize>) -> Result<Vec<Entry>, StorageError> {
        let mut entries = Vec::with_capacity(range.len());
        self.each_entry(connection, range, |entry| {
            entries.push(entry);
            ControlFlow::Continue(())
        })?;
        Ok(entries)
    }

    /// Hands its entries at the indices `range`, all of which it holds, to
    /// `take` in order, decoded one at a time, until `take` breaks, after
    /// which no entry is read.
    fn each_entry(
        &self,
        connection: &Connection,
        range: Range<usize>,
        mut take: impl FnMut(Entry) -> ControlFlow<()>,
    ) -> Result<(), StorageError> {
        let token = match self {
            Self::Agent(node) => {
                let path = Path::read(connection, node.as_str())?;
                return each_block(connection, node, &path, count(range.start)..count(range.end), |_, block| {
                    take(Entry::Block(block))
                });
            }
            Self::Shared { token } => token,
        };
        let mut statement = connection.prepare(&format!(
            "SELECT entry.position, entry.steps, {} FROM {LISTED}
             WHERE entry.token = ?1 AND entry.position >= ?2 AND entry.position < ?3
             ORDER BY entry.position",
            listed_record()
        ))?;
        let codec = Codec::new(connection);
        let mut rows = statement.query(params![token.as_str(), count(range.start), count(range.end)])?;
        let mut at = range.start;
        while let Some(row) = rows.next()? {
            let index: i64 = row.get(0)?;
            // A share's entry whose row or copy is gone holds no block.
            let entry = match (index == count(at), listed(row, 2)?, row.get::<_, Option<i64>>(1)?) {
                (true, Some(stored), None) => Entry::Block(codec.block(self.table(), &stored)?),
                (true, None, Some(steps)) => {
                    Entry::StepsNotShared(decode("share_entries", "steps", u64::try_from(steps))?)
                }
                _ => return Err(self.gap(at)),
            };
            at += 1;
            if take(entry).is_break() {
                return Ok(());
            }
        }
        if at != range.end {
            return Err(self.gap(at));
        }
        Ok(())
    }

    /// The type, the id and the summary size of the block of its entry at
    /// `index`, which it holds, read from the columns of its row without
    /// decoding the block: NULL for what the block lacks, and for each of
    /// a run of steps. A share's copy, always a text, has no summary size.
    fn columns_at(&self, connection: &Connection, index: usize) -> Result<[Value; 3], StorageError> {
        let (sql, key) = match self {
            Self::Agent(node) => {
                let path = Path::read(connection, node.as_str())?;
                let Some(Segment { node: holder, rows, .. }) =
                    path.segments(node.as_str(), count(index)..count(index) + 1).pop()
                else {
                    return Err(self.gap(index));
                };
                (
                    "SELECT type, id, summary_tokens FROM blocks WHERE node_id = ?1 AND idx = ?2".to_owned(),
                    [Value::Text(holder.to_owned()), Value::Integer(rows.start)],
                )
            }
            Self::Shared { token } => (
                // An entry whose row or copy is gone, which holds no block,
                // reads as missing.
                format!(
                    "SELECT coalesce(row.type, copy.type), coalesce(row.id, copy.id), row.summary_tokens
                     FROM {LISTED} WHERE entry.token = ?1 AND entry.position = ?2
                       AND (row.record IS NOT NULL OR copy.copy IS NOT NULL OR entry.steps IS NOT NULL)"
                ),
                [Value::Text(token.as_str().to_owned()), Value::Integer(count(index))],
            ),
        };
        connection
            .query_row(&sql, params_from_iter(key), |row| Ok([row.get(0)?, row.get(1)?, row.get(2)?]))
            .optional()?
            .ok_or_else(|| self.gap(index))
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
        let agent_sql = format!(
            "SELECT {POSITION} FROM blocks INDEXED BY blocks_by_id CROSS JOIN path_runs AS run
               ON run.node_id = ?1 AND {RUN_NODE} = blocks.node_id
                 AND blocks.idx >= run.first AND blocks.idx < run.first + run.length
             WHERE blocks.id = ?2 AND {POSITION} < ?3"
        );
        let (sql, parameters): (&str, Vec<Value>) = match self {
            Self::Agent(node) => (
                // The id of a block that a rewrite moved is on its old
                // branch too; the path holds it once.
                &agent_sql,
                [&[Value::Text(node.as_str().to_owned())][..], &wanted].concat(),
            ),
            Self::Shared { token } => (
                // From the rows of that id to the entry that names one.
                "SELECT entry.position FROM blocks AS row INDEXED BY blocks_by_id
                   CROSS JOIN share_entries AS entry INDEXED BY share_entries_by_row
                     ON entry.token = ?1 AND entry.node_id = row.node_id AND entry.idx = row.idx
                 WHERE row.id = ?2 AND entry.position < ?3
                 UNION ALL
                 SELECT entry.position FROM share_blocks AS copy
                   JOIN share_entries AS entry ON entry.token = copy.token AND entry.position = copy.idx
                 WHERE copy.token = ?1 AND copy.id = ?2
                   AND entry.node_id IS NULL AND entry.position < ?3
                 LIMIT 1",
                [&[Value::Text(token.as_str().to_owned())][..], &wanted].concat(),
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
    /// `length` blocks, which the context card lists; none before the
    /// first, and none in a share, which shows no `context` block.
    pub fn newest_instructions(
        &self,
        connection: &Connection,
        length: usize,
    ) -> Result<Vec<InstructionEntry>, StorageError> {
        let Self::Agent(node) = self else {
            return Ok(Vec::new());
        };
        let stored = connection
            .query_row(
                &format!(
                    "SELECT {} FROM path_runs AS run CROSS JOIN blocks INDEXED BY blocks_by_type
                       ON blocks.node_id = {RUN_NODE} AND blocks.type = 'context'
                         AND blocks.idx >= run.first AND blocks.idx < run.first + run.length
                     WHERE run.node_id = ?1 AND {POSITION} < ?2 AND blocks.source = ?3
                     ORDER BY {POSITION} DESC LIMIT 1",
                    record_of("blocks")
                ),
                params![node.as_str(), count(length), INSTRUCTIONS_SOURCE],
                |row| Stored::of(row, 0),
            )
            .optional()?;
        let Some(stored) = stored else {
            return Ok(Vec::new());
        };
        match Codec::new(connection).block("blocks", &stored)? {
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
            Self::Shared { token } => StorageError::Corrupt {
                table: "share_entries",
                column: "position",
                reason: format!("share {token} has no entry {index}"),
            },
        }
    }
}

/// What a page read found.
pub enum PageRead {
    /// The page: its first block's index, the transcript's length, its
    /// blocks whole, the entries of the newest instructions block, and the
    /// summary size of each compaction marker's boundary.
    Page {
        start: usize,
        length: usize,
        blocks: Vec<Entry>,
        instructions: Vec<InstructionEntry>,
        summaries: Vec<CompactionSummary>,
        /// The versions of the page's blocks that have others; none in a
        /// share, which shows one path.
        versions: Vec<BlockVersions>,
    },
    NoNode,
    NoBlock,
    /// `before` or `after` an index the transcript does not reach.
    Outside,
    Changed,
    Invalid(&'static str),
}

/// Where a page is read from, as its query asks: none, the latest, with how
/// many of its last `blocks`; `before` or `after` with its `edge`; or `around`.
#[derive(Debug, Clone, Copy)]
pub struct PageQuery<'a> {
    pub blocks: Option<u32>,
    pub before: Option<u32>,
    pub after: Option<u32>,
    pub around: Option<&'a BlockId>,
    pub edge: Option<&'a BlockId>,
}

impl<'a> From<&'a TranscriptQuery> for PageQuery<'a> {
    fn from(query: &'a TranscriptQuery) -> Self {
        Self {
            blocks: query.blocks,
            before: query.before,
            after: query.after,
            around: query.around.as_ref(),
            edge: query.edge.as_ref(),
        }
    }
}

/// The page `query` asks of `transcript`, its blocks whole: a thousand of them,
/// wherever requests begin and end ([`page`]).
pub fn read_page(
    connection: &Connection,
    transcript: &Transcript,
    query: PageQuery<'_>,
) -> Result<PageRead, StorageError> {
    let Some(length) = transcript.length(connection)? else {
        return Ok(PageRead::NoNode);
    };
    // The index a page extends from, while the block there is still the
    // `edge` the page holds; otherwise why the read is refused. The block
    // is outside the page, so only its id is read.
    let edge_at = |index: u32| -> Result<Result<usize, PageRead>, StorageError> {
        let Some(edge) = query.edge else {
            return Ok(Err(PageRead::Invalid("`before` and `after` take an `edge`")));
        };
        let index = usize::try_from(index).unwrap_or(usize::MAX);
        if index >= length {
            return Ok(Err(PageRead::Outside));
        }
        let held = transcript.columns_at(connection, index)?;
        Ok(if matches!(&held[1], Value::Text(id) if id == edge.as_str()) {
            Ok(index)
        } else {
            Err(PageRead::Changed)
        })
    };
    if query.blocks.is_some() && (query.before.is_some() || query.after.is_some() || query.around.is_some()) {
        return Ok(PageRead::Invalid("`blocks` reads the latest page alone"));
    }
    let at = match (query.before, query.after, query.around) {
        (None, None, None) => PageAt::Latest(query.blocks.map_or(PAGE_BLOCKS, |blocks| usize::try_from(blocks).unwrap_or(usize::MAX))),
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
    let range = page(length, at);
    let blocks = if range.is_empty() {
        Vec::new()
    } else {
        transcript.entries_in(connection, range.clone())?
    };
    let summaries = summaries_of(
        connection,
        transcript,
        length,
        blocks.iter().filter_map(|entry| match entry {
            Entry::Block(block) => Some(block),
            Entry::StepsNotShared(_) => None,
        }),
    )?;
    let versions = match transcript {
        Transcript::Agent(node) => {
            let path = Path::read(connection, node.as_str())?;
            let held = path.versions(connection, count(range.start)..count(range.start + blocks.len()))?;
            held.into_iter()
                .map(|(position, versions)| {
                    let at = decode("path_runs", "start", usize::try_from(position))?;
                    // An agent's transcript holds blocks alone.
                    let id = blocks[at - range.start].id().ok_or_else(|| gap(node, at))?;
                    Ok(BlockVersions::new(id.clone(), versions))
                })
                .collect::<Result<_, StorageError>>()?
        }
        Transcript::Shared { .. } => Vec::new(),
    };
    Ok(PageRead::Page {
        start: range.start,
        length,
        blocks,
        instructions: transcript.newest_instructions(connection, length)?,
        summaries,
        versions,
    })
}

/// The summary size of each compaction marker's boundary among `blocks`, a
/// transcript of `length` blocks holding them: a marker's boundary may be on
/// another page, so only its size is read.
fn summaries_of<'a>(
    connection: &Connection,
    transcript: &Transcript,
    length: usize,
    blocks: impl IntoIterator<Item = &'a Block>,
) -> Result<Vec<CompactionSummary>, StorageError> {
    let mut summaries = Vec::new();
    for block in blocks {
        if let Block::CompactionMarker(marker) = block
            && let Some(index) = transcript.block_index(connection, &marker.boundary_id, length)?
            && let [Value::Text(kind), _, tokens] = &transcript.columns_at(connection, index)?
            && kind == "compaction_boundary"
        {
            let tokens = match tokens {
                Value::Integer(tokens) => decode(transcript.table(), "block", u64::try_from(*tokens))?,
                _ => {
                    return Err(StorageError::Corrupt {
                        table: transcript.table(),
                        column: "block",
                        reason: format!("compaction boundary {} has no summary size", marker.boundary_id),
                    });
                }
            };
            summaries.push(CompactionSummary {
                marker: marker.id.clone(),
                summary_tokens: tokens,
            });
        }
    }
    Ok(summaries)
}

/// What a page of `blocks` of `transcript` carries beside them, read from the
/// store alone: the entries of the transcript's newest instructions block and
/// the summary size of each compaction marker's boundary among `blocks`. It is
/// what a page read from a live window lacks.
pub fn read_context(
    connection: &Connection,
    transcript: &Transcript,
    blocks: &[Block],
) -> Result<(Vec<InstructionEntry>, Vec<CompactionSummary>), StorageError> {
    let Some(length) = transcript.length(connection)? else {
        return Ok((Vec::new(), Vec::new()));
    };
    Ok((
        transcript.newest_instructions(connection, length)?,
        summaries_of(connection, transcript, length, blocks)?,
    ))
}

/// What a read of one block whole found.
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
    match transcript.entries_in(connection, index..index + 1)?.remove(0) {
        Entry::Block(block) => Ok(BlockRead::Block(block)),
        // An entry found by a block's id holds that block.
        Entry::StepsNotShared(_) => Ok(BlockRead::NoBlock),
    }
}

/// A subagent as [`subagents`] reads it: its record, whether the call that
/// started it is on its parent's current path, and whether its every ancestor's
/// start is too.
///
/// The last makes it a child of the conversation's current path. A child no call
/// started is on every path.
#[derive(Debug, Clone)]
pub struct Subagent {
    pub record: NodeRecord,
    pub started_on_path: bool,
    pub on_path: bool,
}

/// Whether the call that started the node `nodes` names is on its parent's
/// current path.
const STARTED_ON_PATH: &str = "(nodes.started_by IS NULL OR EXISTS (
     SELECT 1 FROM blocks AS start INDEXED BY blocks_by_id CROSS JOIN path_runs AS run
       ON run.node_id = start.node_id AND start.idx >= run.first AND start.idx < run.first + run.length
     WHERE start.id = nodes.started_by AND start.node_id = nodes.parent_id
   ))";

/// The query [`subagents`] runs.
fn subagents_sql() -> String {
    format!(
        "WITH RECURSIVE on_path (id) AS (
           SELECT id FROM nodes WHERE parent_id IS NULL
           UNION ALL
           SELECT nodes.id FROM nodes JOIN on_path ON nodes.parent_id = on_path.id WHERE {STARTED_ON_PATH}
         )
         SELECT {NODE_COLUMNS},
           {STARTED_ON_PATH} AS started_on_path, id IN (SELECT id FROM on_path) AS on_path
         FROM {NODES} WHERE parent_id IS NOT NULL AND number IS NOT NULL ORDER BY number"
    )
}

/// The subagents of the tree, in the order they started.
pub fn subagents(connection: &Connection) -> Result<Vec<Subagent>, StorageError> {
    let mut statement = connection.prepare(&subagents_sql())?;
    let mut rows = statement.query([])?;
    let mut nodes = Vec::new();
    while let Some(row) = rows.next()? {
        nodes.push(Subagent {
            record: node_row(row)?,
            started_on_path: row.get("started_on_path")?,
            on_path: row.get("on_path")?,
        });
    }
    Ok(nodes)
}

/// The call that started the command `command`, with the node that ran it, on
/// whatever branch it is, since the command ran.
///
/// The call is chosen from the calls whose view names the command:
///
/// - The first, since a stored look at a command, from before such looks were
///   removed, came after it.
/// - Of calls equally early, the oldest agent's, since a fork's copy of its
///   source's call keeps the call's time
///   (`crates/agent-server/src/server/tree/supervisor.rs`), and a side question's
///   copy comes after every agent's.
pub fn command_call(
    connection: &Connection,
    command: &CommandId,
) -> Result<Option<(NodeId, ToolCallBlock)>, StorageError> {
    let row: Option<(String, Stored)> = connection
        .query_row(
            "SELECT blocks.node_id, blocks.format, blocks.dictionary, blocks.body FROM blocks JOIN nodes ON nodes.id = blocks.node_id
             WHERE blocks.command = ?1
             ORDER BY blocks.created_at, nodes.number IS NULL, nodes.number LIMIT 1",
            [command.as_str()],
            |row| Ok((row.get(0)?, Stored::of(row, 1)?)),
        )
        .optional()?;
    let Some((node, stored)) = row else {
        return Ok(None);
    };
    match Codec::new(connection).block("blocks", &stored)? {
        Block::ToolCall(call) => Ok(Some((decode("blocks", "node_id", NodeId::try_from(node))?, call))),
        _ => Err(StorageError::Corrupt {
            table: "blocks",
            column: "block",
            reason: format!("a block that is not a call names command {command}"),
        }),
    }
}
