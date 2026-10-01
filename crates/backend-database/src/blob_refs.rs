//! The `blob_refs` index of a conversation's database (`storage.md`
//! § Conversation state and transactions, § Retention): one row for each
//! blob a block references, with its node, its block's index, the
//! reference's place in the block, its blob, what in the block holds it (a
//! message's medium, a tool result's medium or an edit copy), and the
//! block's time. The rows are derived from the blocks by [`rows`]: every
//! write of a block goes through [`write_block`], and every removal through
//! [`truncate`] or a node's deletion, in the transaction that changes the
//! block, and nothing writes the rows on their own. The blobs whose rows a
//! commit writes or removes are its uses, which it records before it commits
//! (`storage.md` § Collecting blobs).
//!
//! The retention pass reads the index: [`retirable`] and [`retire`] apply
//! the agent's rule to the nodes whose tool media have expired, and
//! [`references`] answers every blob the tree names.

use std::collections::BTreeSet;

use demi_agent_store::media::{self, BlobStore, BlockReference, Holder};
use demi_agent_store::{CheckpointState, StoreError};
use demi_agent_transcript::retire::{self as rule, KEPT, Retirement};
use demi_shared_types::{BlobRef, Block, NodeId, Timestamp};
use rusqlite::{Connection, Transaction, params};

use super::StorageError;
use super::command_outputs;
use super::columns::{count, decode, json, to_json};
use super::tree::blocks_of;

/// The owner's blob namespace as the conversation database's commits use it
/// (`storage.md` § Collecting blobs): where a session's media are stored and
/// read, and the record of blob uses, which each commit that writes or
/// removes a reference updates before it commits. The object store keeps
/// both; storage reaches them through this trait, since it calls no other
/// backend crate.
pub trait OwnerBlobs: Send + Sync {
    /// The namespace, as a session reaches it through its tree store.
    fn media(&self) -> &dyn BlobStore;

    /// Records that a commit writes or removes references to `blobs`,
    /// inside its transaction and before it commits. It is refused when one
    /// of them is being deleted, and the commit must then not commit.
    fn commit_uses(&self, blobs: &[BlobRef]) -> Result<(), StoreError>;
}

/// One row of the index: a blob one block references.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BlobRefRow {
    /// The reference's place among the block's references.
    pub part: usize,
    pub blob: BlobRef,
    pub holder: Holder,
    /// The block's time.
    pub at: Timestamp,
}

/// The index rows of `block`: the one derivation of them.
pub fn rows(block: &Block) -> Vec<BlobRefRow> {
    let at = block.created_at();
    media::block_references(block)
        .into_iter()
        .enumerate()
        .map(|(part, BlockReference { blob, holder })| BlobRefRow {
            part,
            blob: blob.clone(),
            holder,
            at,
        })
        .collect()
}

/// The `holder` column's value for `holder`.
pub fn holder_name(holder: Holder) -> &'static str {
    match holder {
        Holder::Message => "message",
        Holder::ToolResult => "tool_result",
        Holder::EditCopy => "edit_copy",
    }
}

/// Writes `block` at `index` of `node` with its index rows, in place of
/// what the index held there. The blobs of the rows it removes and writes
/// join `touched`.
pub fn write_block(
    transaction: &Transaction<'_>,
    node: &NodeId,
    index: usize,
    block: &Block,
    touched: &mut Vec<BlobRef>,
) -> Result<(), StorageError> {
    let index = count(index);
    transaction
        .prepare_cached(
            "INSERT INTO blocks (node_id, idx, block) VALUES (?1, ?2, ?3)
             ON CONFLICT (node_id, idx) DO UPDATE SET block = excluded.block",
        )?
        .execute(params![node.as_str(), index, to_json(block)])?;
    removed(
        transaction,
        "DELETE FROM blob_refs WHERE node_id = ?1 AND idx = ?2 RETURNING blob",
        node,
        index,
        touched,
    )?;
    let mut insert = transaction.prepare_cached(
        "INSERT INTO blob_refs (node_id, idx, part, blob, holder, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for row in rows(block) {
        insert.execute(params![
            node.as_str(),
            index,
            count(row.part),
            row.blob.as_str(),
            holder_name(row.holder),
            row.at.as_millisecond()
        ])?;
        touched.push(row.blob);
    }
    Ok(())
}

/// Deletes the blocks of `node` from `index` on, with their index rows,
/// whose blobs join `touched`.
pub fn truncate(
    transaction: &Transaction<'_>,
    node: &NodeId,
    index: usize,
    touched: &mut Vec<BlobRef>,
) -> Result<(), StorageError> {
    let index = count(index);
    removed(
        transaction,
        "DELETE FROM blob_refs WHERE node_id = ?1 AND idx >= ?2 RETURNING blob",
        node,
        index,
        touched,
    )?;
    transaction
        .prepare_cached("DELETE FROM blocks WHERE node_id = ?1 AND idx >= ?2")?
        .execute(params![node.as_str(), index])?;
    Ok(())
}

/// The blobs the index rows of `node` and its descendants name, which the
/// node's deletion removes.
pub fn subtree(transaction: &Transaction<'_>, node: &NodeId) -> Result<Vec<BlobRef>, StorageError> {
    let mut statement = transaction.prepare(
        "WITH RECURSIVE subtree (id) AS (
           SELECT ?1 UNION SELECT nodes.id FROM nodes JOIN subtree ON nodes.parent_id = subtree.id
         )
         SELECT blob FROM blob_refs WHERE node_id IN subtree",
    )?;
    let mut rows = statement.query([node.as_str()])?;
    let mut blobs = Vec::new();
    while let Some(row) = rows.next()? {
        blobs.push(blob(row.get(0)?)?);
    }
    Ok(blobs)
}

/// Runs a deletion of index rows that answers each row's blob, and adds
/// them to `touched`.
fn removed(
    transaction: &Transaction<'_>,
    deletion: &str,
    node: &NodeId,
    index: i64,
    touched: &mut Vec<BlobRef>,
) -> Result<(), StorageError> {
    let mut statement = transaction.prepare_cached(deletion)?;
    let mut rows = statement.query(params![node.as_str(), index])?;
    while let Some(row) = rows.next()? {
        touched.push(blob(row.get(0)?)?);
    }
    Ok(())
}

/// The blocks the agent's rule retires in the tree `connection` holds, by
/// node (`runtime.md` § Retired tool media): only the nodes whose index
/// holds a tool result's medium older than [`KEPT`] are read.
pub fn retirable(
    connection: &Connection,
    retirement: Retirement,
) -> Result<Vec<(NodeId, Vec<(usize, Block)>)>, StorageError> {
    // Thirty days before any time a clock gives is a time too.
    let expired = retirement
        .now
        .to_jiff()
        .checked_sub(KEPT)
        .map_or(i64::MIN, |time| time.as_millisecond());
    let mut statement = connection.prepare(
        "SELECT id, block_count FROM nodes
         WHERE id IN (SELECT node_id FROM blob_refs WHERE holder = ?1 AND at < ?2)
         ORDER BY id",
    )?;
    let mut nodes = statement.query(params![holder_name(Holder::ToolResult), expired])?;
    let mut retired = Vec::new();
    while let Some(row) = nodes.next()? {
        let node = decode("nodes", "id", NodeId::try_from(row.get::<_, String>(0)?))?;
        let blocks = blocks_of(connection, &node, row.get(1)?)?;
        let changed = rule::retire(&blocks, retirement);
        if !changed.is_empty() {
            retired.push((node, changed));
        }
    }
    Ok(retired)
}

/// Retires the expired tool media of the tree `connection` holds, in one
/// transaction (`storage.md` § Retiring tool media): each block the rule
/// changes is written in place with its index rows. No node's state row,
/// block count or output revision changes, so the conversation does not
/// show as unread. The blobs whose references go are used now, before the
/// commit, so a collection keeps them for another grace. Answers how many
/// blocks changed.
pub fn retire(
    connection: &mut Connection,
    blobs: &dyn OwnerBlobs,
    retirement: Retirement,
) -> Result<Result<usize, StoreError>, StorageError> {
    let transaction = connection.transaction()?;
    let retired = retirable(&transaction, retirement)?;
    let mut touched = Vec::new();
    let mut changed = 0;
    for (node, blocks) in &retired {
        for (index, block) in blocks {
            write_block(&transaction, node, *index, block, &mut touched)?;
            changed += 1;
        }
    }
    if let Err(refused) = blobs.commit_uses(&touched) {
        return Ok(Err(refused));
    }
    transaction.commit()?;
    Ok(Ok(changed))
}

/// Every blob the conversation `connection` holds a reference to: its
/// blocks' media and edit copies, as the index names them, its nodes' queued
/// messages', and its commands' outputs'.
pub fn references(connection: &Connection) -> Result<BTreeSet<BlobRef>, StorageError> {
    let mut references: BTreeSet<BlobRef> = command_outputs::references(connection)?.into_iter().collect();
    let mut statement = connection.prepare("SELECT DISTINCT blob FROM blob_refs")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        references.insert(blob(row.get(0)?)?);
    }
    let mut statement = connection.prepare("SELECT state FROM nodes")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let state: CheckpointState = json("nodes", "state", &row.get::<_, String>(0)?)?;
        for message in &state.queue {
            references.extend(media::content_references(&message.content).cloned());
        }
    }
    Ok(references)
}

fn blob(text: String) -> Result<BlobRef, StorageError> {
    decode("blob_refs", "blob", BlobRef::try_from(text))
}
