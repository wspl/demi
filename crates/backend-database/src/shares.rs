//! Shares (`sharing.md` § Storage and routes): each share's record in the
//! control database, which the token finds, and its list in its
//! conversation's database, the blocks it shows by the transcript rows that
//! hold them or by its copies of the few still changing, which a share's
//! pages read as a conversation's pages read its transcript
//! ([`tree::Transcript::Shared`]). A record goes with its conversation; the
//! list goes with the conversation's database.

use std::collections::BTreeSet;

use demi_agent_store::media::block_blobs;
use demi_shared_types::{BlobRef, Block, BlockId, NodeId, Timestamp};
use demi_web_api_protocol::ids::{ConversationId, ShareToken, UserId};
use demi_web_api_protocol::shares::ShareTarget;
use comrak::nodes::NodeValue;
use comrak::{Arena, Options, parse_document};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use super::StorageError;
use super::columns::{decode, instant, json, to_json};
use super::control::ControlService;
use super::conversation_attachments::{self, AttachmentNumber, AttachmentRow};

/// A share's record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareRecord {
    pub token: ShareToken,
    pub owner: UserId,
    pub conversation: ConversationId,
    pub target: ShareTarget,
    /// The conversation's title when the list was made; none while it
    /// was the placeholder.
    pub title: Option<String>,
    pub taken_at: Timestamp,
}

/// What adding a share found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Added {
    /// The share was added.
    New(ShareRecord),
    /// The conversation has a share of that target already, which stays.
    Existing(ShareRecord),
    /// The conversation is gone.
    NoConversation,
}

const SHARE_COLUMNS: &str = "token, user_id, conversation_id, kind, block_id, title, taken_at";

fn share_row(row: &Row<'_>) -> Result<ShareRecord, StorageError> {
    let kind: String = row.get("kind")?;
    let block: Option<String> = row.get("block_id")?;
    let target = match (kind.as_str(), block) {
        ("conversation", None) => ShareTarget::Conversation,
        ("answer", Some(block)) => ShareTarget::Answer {
            block_id: decode("shares", "block_id", BlockId::try_from(block))?,
        },
        (kind, _) => {
            return Err(StorageError::Corrupt {
                table: "shares",
                column: "kind",
                reason: format!("a share of kind {kind} with the block it holds"),
            });
        }
    };
    Ok(ShareRecord {
        token: decode("shares", "token", ShareToken::try_from(row.get::<_, String>("token")?))?,
        owner: decode("shares", "user_id", UserId::try_from(row.get::<_, String>("user_id")?))?,
        conversation: decode(
            "shares",
            "conversation_id",
            ConversationId::try_from(row.get::<_, String>("conversation_id")?),
        )?,
        target,
        title: row.get("title")?,
        taken_at: instant(row, "shares", "taken_at")?,
    })
}

/// The kind and block columns of `target`.
fn target_columns(target: &ShareTarget) -> (&'static str, Option<&str>) {
    match target {
        ShareTarget::Conversation => ("conversation", None),
        ShareTarget::Answer { block_id } => ("answer", Some(block_id.as_str())),
    }
}

fn share_of(
    connection: &Connection,
    conversation: &ConversationId,
    target: &ShareTarget,
) -> Result<Option<ShareRecord>, StorageError> {
    let (kind, block) = target_columns(target);
    connection
        .query_row(
            &format!(
                "SELECT {SHARE_COLUMNS} FROM shares
                 WHERE conversation_id = ?1 AND kind = ?2 AND block_id IS ?3"
            ),
            params![conversation.as_str(), kind, block],
            |row| Ok(share_row(row)),
        )
        .optional()?
        .transpose()
}

impl ControlService {
    /// The share that answers `token`.
    pub async fn share(&self, token: ShareToken) -> Result<Option<ShareRecord>, StorageError> {
        self.call(move |connection, _| {
            connection
                .query_row(
                    &format!("SELECT {SHARE_COLUMNS} FROM shares WHERE token = ?1"),
                    [token.as_str()],
                    |row| Ok(share_row(row)),
                )
                .optional()?
                .transpose()
        })
        .await
    }

    /// The conversation's share of `target`, if it has one.
    pub async fn share_of(
        &self,
        conversation: ConversationId,
        target: ShareTarget,
    ) -> Result<Option<ShareRecord>, StorageError> {
        self.call(move |connection, _| share_of(connection, &conversation, &target))
            .await
    }

    /// The conversation's shares, its own first, then its answers' in the
    /// order they were taken.
    pub async fn shares_of(&self, conversation: ConversationId) -> Result<Vec<ShareRecord>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare(&format!(
                "SELECT {SHARE_COLUMNS} FROM shares WHERE conversation_id = ?1
                 ORDER BY kind = 'answer', taken_at, token"
            ))?;
            let mut rows = statement.query([conversation.as_str()])?;
            let mut shares = Vec::new();
            while let Some(row) = rows.next()? {
                shares.push(share_row(row)?);
            }
            Ok(shares)
        })
        .await
    }

    /// Adds `share`, unless its conversation has a share of its target
    /// already or is gone.
    pub async fn add_share(&self, share: ShareRecord) -> Result<Added, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            if let Some(existing) = share_of(&transaction, &share.conversation, &share.target)? {
                return Ok(Added::Existing(existing));
            }
            let (kind, block) = target_columns(&share.target);
            let added = transaction.execute(
                "INSERT INTO shares (token, user_id, conversation_id, kind, block_id, title, taken_at)
                 SELECT ?1, ?2, id, ?3, ?4, ?5, ?6 FROM conversations WHERE id = ?7 AND user_id = ?2",
                params![
                    share.token.as_str(),
                    share.owner.as_str(),
                    kind,
                    block,
                    share.title,
                    share.taken_at.as_millisecond(),
                    share.conversation.as_str(),
                ],
            )?;
            transaction.commit()?;
            Ok(if added == 1 { Added::New(share) } else { Added::NoConversation })
        })
        .await
    }

    /// Records that the share's list was made again, with the title the
    /// conversation had then; none when the share is gone.
    pub async fn retake_share(
        &self,
        token: ShareToken,
        title: Option<String>,
        taken_at: Timestamp,
    ) -> Result<Option<ShareRecord>, StorageError> {
        self.call(move |connection, _| {
            connection
                .query_row(
                    &format!(
                        "UPDATE shares SET title = ?2, taken_at = ?3 WHERE token = ?1
                         RETURNING {SHARE_COLUMNS}"
                    ),
                    params![token.as_str(), title, taken_at.as_millisecond()],
                    |row| Ok(share_row(row)),
                )
                .optional()?
                .transpose()
        })
        .await
    }

    /// Removes the conversation's share `token`; false when it has none.
    /// From this commit on, no request finds it.
    pub async fn remove_share(
        &self,
        conversation: ConversationId,
        token: ShareToken,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let removed = connection.execute(
                "DELETE FROM shares WHERE token = ?1 AND conversation_id = ?2",
                params![token.as_str(), conversation.as_str()],
            )?;
            Ok(removed == 1)
        })
        .await
    }
}

/// The block types a conversation's share leaves out: the `context` blocks,
/// the hidden inputs that carry the execution context, instructions and
/// skills, and the `response` blocks, the usage of a request. The page
/// hides the rest as the conversation's page does.
const HIDDEN: &str = "('context', 'response')";

/// Makes the list of the conversation's share `token` (`sharing.md`
/// § Storage and routes), in place of one it had: every block each agent's
/// transcript shows, the root's and each subagent's, in order, by the row
/// that holds it. The list is made from the transcript's index of block
/// types, without reading a block: only the blocks still changing are read,
/// to copy them, and the texts that name an attachment.
///
/// A block still changing is one of the request a node is making or of the
/// calls of the one before, since a request's `response` block follows its
/// blocks and its calls run after it: a block after the node's last
/// `response`, which the model may still be writing, and a call after the
/// one before that still executing, whose result is still to come. Its
/// entry points at its copy, as it stands, without the vendor's record of
/// the session, which never leaves the backend.
pub fn take_conversation(transaction: &Transaction<'_>, token: &ShareToken) -> Result<(), StorageError> {
    remove(transaction, token)?;
    transaction.execute(
        &format!(
            "INSERT INTO share_entries (token, agent, position, node_id, idx)
             SELECT ?1, nodes.number, row_number() OVER (PARTITION BY blocks.node_id ORDER BY blocks.idx) - 1,
                    blocks.node_id, blocks.idx
             FROM nodes CROSS JOIN blocks INDEXED BY blocks_by_type ON blocks.node_id = nodes.id
             WHERE blocks.idx < nodes.block_count AND json_extract(blocks.block, '$.type') NOT IN {HIDDEN}"
        ),
        [token.as_str()],
    )?;
    transaction.execute(
        "WITH responses AS (
           SELECT nodes.id AS node_id,
             (SELECT idx FROM blocks WHERE node_id = nodes.id AND json_extract(block, '$.type') = 'response'
                AND idx < nodes.block_count ORDER BY idx DESC LIMIT 1) AS last,
             (SELECT idx FROM blocks WHERE node_id = nodes.id AND json_extract(block, '$.type') = 'response'
                AND idx < nodes.block_count ORDER BY idx DESC LIMIT 1 OFFSET 1) AS before_last
           FROM nodes
         )
         INSERT INTO share_blocks (token, agent, idx, block)
         SELECT entry.token, entry.agent, entry.position, json_remove(blocks.block, '$.entries')
         FROM responses
         JOIN share_entries AS entry
           ON entry.token = ?1 AND entry.node_id = responses.node_id AND entry.idx > coalesce(responses.before_last, -1)
         JOIN blocks ON blocks.node_id = entry.node_id AND blocks.idx = entry.idx
         WHERE entry.idx > coalesce(responses.last, -1) OR json_extract(blocks.block, '$.status') = 'executing'",
        [token.as_str()],
    )?;
    // The attachments its texts name, through `block_attachments`, while
    // every entry still names its row.
    keep_attachments(
        transaction,
        "SELECT named.attachment FROM share_entries AS entry
           JOIN block_attachments AS named ON named.node_id = entry.node_id AND named.idx = entry.idx
         WHERE entry.token = ?1",
        params![token.as_str()],
    )?;
    point_at_copies(transaction, token)?;
    name_copied_blobs(transaction, token)?;
    // Each subagent with the description and profile it has now.
    transaction.execute(
        "INSERT INTO share_agents (token, number, description, profile)
         SELECT ?1, number, description, profile FROM nodes WHERE parent_id IS NOT NULL",
        [token.as_str()],
    )?;
    Ok(())
}

/// Lists in `share_blobs` the blobs the copies of `token` name, the few
/// blocks it holds of its own.
fn name_copied_blobs(transaction: &Transaction<'_>, token: &ShareToken) -> Result<(), StorageError> {
    let mut copies = transaction.prepare("SELECT block FROM share_blocks WHERE token = ?1")?;
    let mut name = transaction.prepare("INSERT OR IGNORE INTO share_blobs (token, blob) VALUES (?1, ?2)")?;
    let mut rows = copies.query([token.as_str()])?;
    while let Some(row) = rows.next()? {
        let block: Block = json("share_blocks", "block", &row.get::<_, String>(0)?)?;
        for blob in block_blobs(&block) {
            name.execute(params![token.as_str(), blob.as_str()])?;
        }
    }
    Ok(())
}

/// Which listed rows a write is about to replace or remove: one row, the
/// rows of a node from an index on, or every row of a node and of its
/// descendants.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Rewritten<'a> {
    Row(&'a NodeId, i64),
    From(&'a NodeId, i64),
    Subtree(&'a NodeId),
}

/// Keeps what every share shows of the rows `rewritten` names before a
/// write replaces or removes them (`sharing.md` § Storage and routes): each
/// listed row is copied into the share as it stands, with the blobs it
/// names, and its entry points at the copy, in the write's transaction. The
/// one guard of every write to `blocks` that changes a row.
pub(crate) fn keep_rewritten(transaction: &Transaction<'_>, rewritten: Rewritten<'_>) -> Result<(), StorageError> {
    let (rows, node, index) = match rewritten {
        Rewritten::Row(node, index) => ("entry.node_id = ?1 AND entry.idx = ?2", node, index),
        Rewritten::From(node, index) => ("entry.node_id = ?1 AND entry.idx >= ?2", node, index),
        Rewritten::Subtree(node) => (
            "entry.node_id IN (
               WITH RECURSIVE subtree (id) AS (
                 SELECT ?1 UNION ALL SELECT nodes.id FROM nodes JOIN subtree ON nodes.parent_id = subtree.id
               ) SELECT id FROM subtree
             ) AND ?2 IS NOT NULL",
            node,
            0,
        ),
    };
    let parameters = params![node.as_str(), index];
    transaction
        .prepare_cached(&format!(
            "INSERT INTO share_blocks (token, agent, idx, block)
             SELECT entry.token, entry.agent, entry.position, json_remove(row.block, '$.entries')
             FROM share_entries AS entry JOIN blocks AS row ON row.node_id = entry.node_id AND row.idx = entry.idx
             WHERE {rows}"
        ))?
        .execute(parameters)?;
    transaction
        .prepare_cached(&format!(
            "INSERT OR IGNORE INTO share_blobs (token, blob)
             SELECT entry.token, named.blob
             FROM share_entries AS entry JOIN block_blobs AS named ON named.node_id = entry.node_id AND named.idx = entry.idx
             WHERE {rows}"
        ))?
        .execute(parameters)?;
    transaction
        .prepare_cached(&format!(
            "UPDATE share_entries AS entry SET node_id = NULL, idx = NULL WHERE {rows}"
        ))?
        .execute(parameters)?;
    Ok(())
}

/// Points the entries of the copies of `token` at them.
fn point_at_copies(transaction: &Transaction<'_>, token: &ShareToken) -> Result<(), StorageError> {
    transaction.execute(
        "UPDATE share_entries SET node_id = NULL, idx = NULL
         FROM share_blocks AS copy
         WHERE share_entries.token = ?1 AND copy.token = ?1
           AND copy.agent = share_entries.agent AND copy.idx = share_entries.position",
        [token.as_str()],
    )?;
    Ok(())
}

/// Makes the list of the root's answer `block` under `token`: the user's
/// message that starts its request and the text, both copied. False, and
/// nothing made, when `block` is not a complete text of the root's: the
/// page offers Share only on the text that ends a reply, and which text
/// does is the page's to judge (`web-api.md` § Sharing).
pub fn take_answer(
    transaction: &Transaction<'_>,
    token: &ShareToken,
    block: &BlockId,
) -> Result<bool, StorageError> {
    let root: Option<(String, i64)> = transaction
        .query_row(
            "SELECT id, block_count FROM nodes WHERE parent_id IS NULL",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((root, block_count)) = root else {
        return Ok(false);
    };
    let found: Option<(i64, String)> = transaction
        .query_row(
            "SELECT idx, block FROM blocks WHERE json_extract(block, '$.id') = ?1 AND node_id = ?2 AND idx < ?3",
            params![block.as_str(), root, block_count],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((index, answer)) = found else {
        return Ok(false);
    };
    let answer: Block = json("blocks", "block", &answer)?;
    let Block::Text(text) = &answer else {
        return Ok(false);
    };
    if !text.forkable {
        return Ok(false);
    }
    let message: Option<String> = transaction
        .query_row(
            "SELECT block FROM blocks WHERE node_id = ?1 AND json_extract(block, '$.type') = 'user' AND idx < ?2
             ORDER BY idx DESC LIMIT 1",
            params![root, index],
            |row| row.get(0),
        )
        .optional()?;
    let Some(message) = message else {
        return Ok(false);
    };
    let message: Block = json("blocks", "block", &message)?;
    remove(transaction, token)?;
    keep_attachments(
        transaction,
        "SELECT attachment FROM block_attachments WHERE node_id = ?2 AND idx = ?3",
        params![token.as_str(), root, index],
    )?;
    copy(transaction, token, 0, [&message, &answer])?;
    name_copied_blobs(transaction, token)?;
    Ok(true)
}

/// Copies `blocks` as the list of agent `agent` of `token`, its whole list.
fn copy<'a>(
    transaction: &Transaction<'_>,
    token: &ShareToken,
    agent: i64,
    blocks: impl IntoIterator<Item = &'a Block>,
) -> Result<(), StorageError> {
    let mut statement = transaction
        .prepare_cached("INSERT INTO share_blocks (token, agent, idx, block) VALUES (?1, ?2, ?3, ?4)")?;
    let mut entry = transaction.prepare_cached(
        "INSERT INTO share_entries (token, agent, position, node_id, idx) VALUES (?1, ?2, ?3, NULL, NULL)",
    )?;
    for (index, block) in blocks.into_iter().enumerate() {
        // The vendor's record of the session never leaves the backend, and
        // a share has no session to resume.
        let mut block = block.clone();
        if let Some(entries) = block.entries_mut() {
            entries.clear();
        }
        let index = super::columns::count(index);
        statement.execute(params![token.as_str(), agent, index, to_json(&block)])?;
        entry.execute(params![token.as_str(), agent, index])?;
    }
    Ok(())
}

/// Copies into the share whose token is `?1` of `parameters` each
/// attachment the query `named` lists that exists now, so one uploaded later stays out until the list is made
/// again; its blob stays shared.
fn keep_attachments(
    transaction: &Transaction<'_>,
    named: &str,
    parameters: &[&dyn rusqlite::ToSql],
) -> Result<(), StorageError> {
    transaction.execute(
        &format!(
            "INSERT OR IGNORE INTO share_attachments (token, number, name, media_type, size, width, height, blob)
             SELECT ?1, number, name, media_type, size, width, height, blob FROM attachments
             WHERE number IN ({named})"
        ),
        parameters,
    )?;
    Ok(())
}

/// The attachments a text names by a link's or an image's target
/// `attachment:a3`, as the web app's renderer reads them: the target,
/// trimmed, is `attachment:` and the number whole (`attachmentId` in
/// `packages/web-ui/src/markdown/filePath.ts`).
pub fn named_attachments(text: &str) -> BTreeSet<AttachmentNumber> {
    let arena = Arena::new();
    let mut options = Options::default();
    options.extension.autolink = true;
    let root = parse_document(&arena, text, &options);
    root.descendants()
        .filter_map(|node| match &node.data.borrow().value {
            NodeValue::Link(link) | NodeValue::Image(link) => link
                .url
                .trim()
                .strip_prefix("attachment:")
                .and_then(|number| number.parse().ok()),
            _ => None,
        })
        .collect()
}

/// Removes the share `token`'s list and copies.
pub fn remove(connection: &Connection, token: &ShareToken) -> Result<(), StorageError> {
    for table in ["share_entries", "share_blocks", "share_blobs", "share_agents", "share_attachments"] {
        connection.execute(&format!("DELETE FROM {table} WHERE token = ?1"), [token.as_str()])?;
    }
    Ok(())
}

/// Whether the share `token` shows `blob` (`sharing.md` § Safety): a block
/// on its list names it, as a medium or a side of an edit, by
/// `block_blobs` or by its copies' `share_blobs`, or it is the blob of an
/// attachment the share keeps. No block is read.
pub fn shows_blob(connection: &Connection, token: &ShareToken, blob: &BlobRef) -> Result<bool, StorageError> {
    let shown: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM share_attachments WHERE token = ?1 AND blob = ?2
             UNION ALL
             SELECT 1 FROM share_blobs WHERE token = ?1 AND blob = ?2
             UNION ALL
             SELECT 1 FROM block_blobs AS named INDEXED BY block_blobs_by_blob
               CROSS JOIN share_entries AS entry INDEXED BY share_entries_by_row
                 ON entry.token = ?1 AND entry.node_id = named.node_id AND entry.idx = named.idx
             WHERE named.blob = ?2
             LIMIT 1",
            params![token.as_str(), blob.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    Ok(shown.is_some())
}

/// The attachment `number` as the share `token` keeps it.
pub fn shown_attachment(
    connection: &Connection,
    token: &ShareToken,
    number: AttachmentNumber,
) -> Result<Option<AttachmentRow>, StorageError> {
    conversation_attachments::read_shared(connection, token.as_str(), number)
}

/// A subagent the share `token` holds: its number, its spawn's
/// description and its profile, as they were when it was taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedAgent {
    pub number: u64,
    pub description: String,
    pub profile: Option<String>,
}

/// The subagents the share `token` holds, in the order they started.
pub fn shared_agents(connection: &Connection, token: &ShareToken) -> Result<Vec<SharedAgent>, StorageError> {
    let mut statement = connection.prepare(
        "SELECT number, description, profile FROM share_agents WHERE token = ?1 ORDER BY number",
    )?;
    let mut rows = statement.query([token.as_str()])?;
    let mut agents = Vec::new();
    while let Some(row) = rows.next()? {
        agents.push(SharedAgent {
            number: decode("share_agents", "number", u64::try_from(row.get::<_, i64>(0)?))?,
            description: row.get(1)?,
            profile: row.get(2)?,
        });
    }
    Ok(agents)
}

/// Every blob the shares' copies and kept attachments in a conversation's
/// database name, which keeps them while the shares do (`storage.md`
/// § Retention); the other blocks a list shows are the transcript's, which
/// keeps theirs.
pub fn copied_blobs(connection: &Connection) -> Result<BTreeSet<BlobRef>, StorageError> {
    let mut blobs = BTreeSet::new();
    let mut statement = connection.prepare("SELECT blob FROM share_blobs UNION SELECT blob FROM share_attachments")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        blobs.insert(decode("share_blobs", "blob", BlobRef::try_from(row.get::<_, String>(0)?))?);
    }
    Ok(blobs)
}

#[cfg(test)]
mod tests {
    use demi_agent_store::AgentTreeStore as _;
    use demi_agent_store::testing::test_model;
    use demi_shared_types::{BlobRef, Block, Timestamp};
    use demi_web_api_protocol::ids::ShareToken;

    use super::take_conversation;
    use crate::conversations::ConversationStores;
    use crate::tree::tests::{conversation, id, record, reply, response, shell_call, store, update, user};
    use crate::tree::{self, PageQuery, PageRead, Transcript};

    // A share's list of a conversation's transcripts (`sharing.md` § Storage
    // and routes), made and read on a conversation database in a temporary
    // directory; a few milliseconds each.

    fn token() -> ShareToken {
        ShareToken::try_from("s1").unwrap()
    }

    /// A call of `block` still executing: no result yet, no view.
    fn executing_call(block: &str) -> Block {
        serde_json::from_value(serde_json::json!({
            "type": "tool_call",
            "id": block,
            "createdAt": Timestamp::UNIX_EPOCH,
            "model": test_model(),
            "toolUseId": "call-1",
            "toolName": "shell",
            "input": "{}",
            "status": "executing",
            "output": [],
            "endedAt": null,
            "view": null,
        }))
        .unwrap()
    }

    /// The same call returned with `output`.
    fn returned_call(block: &str, output: &str) -> Block {
        serde_json::from_value(serde_json::json!({
            "type": "tool_call",
            "id": block,
            "createdAt": Timestamp::UNIX_EPOCH,
            "model": test_model(),
            "toolUseId": "call-1",
            "toolName": "shell",
            "input": "{}",
            "status": "completed",
            "output": [{ "type": "text", "text": output }],
            "endedAt": Timestamp::UNIX_EPOCH,
            "view": null,
        }))
        .unwrap()
    }

    async fn make_list(stores: &ConversationStores) {
        stores
            .db(&conversation())
            .call(|connection| {
                let transaction = connection.transaction()?;
                take_conversation(&transaction, &token())?;
                transaction.commit()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// The whole of what the share holds of agent `agent`: the latest page,
    /// which these transcripts fit in.
    async fn shared(stores: &ConversationStores, agent: u64) -> Vec<Block> {
        let transcript = Transcript::Shared { token: token(), agent };
        let read = stores
            .read(&conversation(), move |connection| {
                tree::read_page(connection, &transcript, PageQuery { before: None, after: None, around: None, edge: None })
            })
            .await
            .unwrap()
            .unwrap();
        let PageRead::Page { start: 0, length, blocks, .. } = read else {
            panic!("the whole transcript on one page")
        };
        assert_eq!(length, blocks.len());
        blocks
    }

    async fn rows(stores: &ConversationStores, sql: &'static str) -> i64 {
        stores
            .read(&conversation(), move |connection| Ok(connection.query_row(sql, [], |row| row.get(0))?))
            .await
            .unwrap()
            .unwrap()
    }

    fn ids(blocks: &[Block]) -> Vec<String> {
        blocks.iter().map(|block| block.id().as_str().to_owned()).collect()
    }

    // Forty requests, each a message, its answer and the request's usage,
    // and a subagent's one: the list names every shown block where the
    // transcript holds it, and copies none, since none still changes.
    #[tokio::test(flavor = "local")]
    async fn a_long_conversations_share_lists_its_blocks_and_copies_none() {
        let (tree, stores, _data) = store().await;
        let mut blocks = Vec::new();
        for round in 0..40 {
            blocks.extend([user(&format!("u{round}")), reply(&format!("t{round}")), response(&format!("r{round}"))]);
        }
        let count = blocks.len();
        tree.create_node(record("root", None, 0), update(blocks.into_iter().enumerate().collect(), count))
            .await
            .unwrap();
        let child = vec![(0, user("cu")), (1, reply("ct")), (2, response("cr"))];
        tree.create_node(record("child", Some("root"), 1), update(child, 3)).await.unwrap();

        make_list(&stores).await;

        assert_eq!(rows(&stores, "SELECT COUNT(*) FROM share_blocks").await, 0);
        assert_eq!(rows(&stores, "SELECT COUNT(*) FROM share_entries WHERE node_id IS NULL").await, 0);
        let expected: Vec<String> = (0..40).flat_map(|round| [format!("u{round}"), format!("t{round}")]).collect();
        assert_eq!(ids(&shared(&stores, 0).await), expected);
        assert_eq!(ids(&shared(&stores, 1).await), ["cu", "ct"]);
    }

    // The agent runs a call when the link is made; it then returns, and the
    // agent answers. The share shows the call as it stood, still running,
    // and nothing after it.
    #[tokio::test(flavor = "local")]
    async fn a_call_still_running_shows_as_it_stood_when_the_link_was_made() {
        let (tree, stores, _data) = store().await;
        let running = vec![(0, user("u1")), (1, executing_call("c1")), (2, response("r1"))];
        tree.create_node(record("root", None, 0), update(running, 3)).await.unwrap();

        make_list(&stores).await;
        tree.session_store(&id("root"))
            .save(update(vec![(1, returned_call("c1", "later output")), (3, reply("t1")), (4, response("r2"))], 5))
            .await
            .unwrap();

        let blocks = shared(&stores, 0).await;
        assert_eq!(ids(&blocks), ["u1", "c1"]);
        assert_eq!(blocks[1], executing_call("c1"));
    }

    /// A message showing the image `blob`, its id `block`.
    fn message_with_image(block: &str, blob: &BlobRef) -> Block {
        let mut message = serde_json::to_value(user(block)).unwrap();
        message["content"] = serde_json::json!([
            { "type": "image", "source": { "type": "ref", "ref": blob.as_str(), "mediaType": "image/png" } }
        ]);
        serde_json::from_value(message).unwrap()
    }

    /// The text of `block`, a message's or an answer's.
    fn words(block: &Block) -> String {
        match block {
            Block::User(message) => serde_json::to_string(&message.content).unwrap(),
            Block::Text(text) => text.text.clone(),
            other => panic!("{other:?}"),
        }
    }

    // The user edits the last message after sharing: the edit removes the
    // message and what follows, and the new turn writes its blocks at the
    // same indices. The share goes on showing the message and the answer it
    // was made with, the hidden and later blocks at those indices stay out,
    // and the image the old message showed is still the share's and still
    // kept.
    #[tokio::test(flavor = "local")]
    async fn an_edit_after_sharing_leaves_the_share_as_it_was() {
        let (tree, stores, _data) = store().await;
        let blob = BlobRef::try_from("c".repeat(64)).unwrap();
        let first = vec![(0, message_with_image("u1", &blob)), (1, reply("t1")), (2, response("r1"))];
        tree.create_node(record("root", None, 0), update(first, 3)).await.unwrap();
        make_list(&stores).await;
        let before = shared(&stores, 0).await;

        let session = tree.session_store(&id("root"));
        session.save(update(vec![(0, user("u1-edited"))], 1)).await.unwrap();
        session
            .save(update(vec![(1, response("r2")), (2, reply("t2")), (3, response("r3"))], 4))
            .await
            .unwrap();

        let after = shared(&stores, 0).await;
        assert_eq!(ids(&after), ["u1", "t1"]);
        assert_eq!(after.iter().map(words).collect::<Vec<_>>(), before.iter().map(words).collect::<Vec<_>>());
        let (shown, kept) = stores
            .read(&conversation(), move |connection| {
                Ok((
                    super::shows_blob(connection, &token(), &blob)?,
                    crate::references::conversation_blobs(connection)?.contains(&blob),
                ))
            })
            .await
            .unwrap()
            .unwrap();
        assert!(shown && kept, "shown {shown}, kept {kept}");
    }

    // A later call of the same request lists again what an earlier shell
    // call changed, rewriting the earlier call's view after the link was
    // made: the share shows the view as it was.
    #[tokio::test(flavor = "local")]
    async fn a_call_rewritten_after_sharing_shows_its_view_as_it_was() {
        let (tree, stores, _data) = store().await;
        let blocks = vec![(0, user("u1")), (1, shell_call("c1", "1")), (2, response("r1")), (3, reply("t1")), (4, response("r2"))];
        tree.create_node(record("root", None, 0), update(blocks, 5)).await.unwrap();
        make_list(&stores).await;

        let mut relisted = serde_json::to_value(shell_call("c1", "1")).unwrap();
        relisted["view"]["chunks"] = serde_json::json!([{ "stream": "stdout", "text": "written later" }]);
        let relisted: Block = serde_json::from_value(relisted).unwrap();
        tree.session_store(&id("root")).save(update(vec![(1, relisted)], 5)).await.unwrap();

        let blocks = shared(&stores, 0).await;
        assert_eq!(ids(&blocks), ["u1", "c1", "t1"]);
        assert_eq!(blocks[1], shell_call("c1", "1"));
    }

}
