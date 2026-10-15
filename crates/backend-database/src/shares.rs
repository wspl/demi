//! Shares (`sharing.md` § Storage and routes): each share's record in the
//! control database, which the token finds, and its snapshot in its
//! conversation's database, the blocks it holds copied from the transcripts
//! as they were, which a share's pages read as a conversation's pages read
//! its transcript ([`tree::Transcript::Shared`]). A record goes with its
//! conversation; the snapshot goes with the conversation's database.

use std::collections::BTreeSet;

use demi_agent_store::media::block_blobs;
use demi_shared_types::{BlobRef, Block, BlockId, Timestamp};
use demi_web_api_protocol::ids::{ConversationId, ShareToken, UserId};
use demi_web_api_protocol::shares::ShareTarget;
use comrak::nodes::NodeValue;
use comrak::{Arena, Options, parse_document};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use super::StorageError;
use super::columns::{decode, instant, json, to_json};
use super::control::ControlService;
use super::conversation_attachments::{self, AttachmentNumber, AttachmentRow};
use super::tree;

/// A share's record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareRecord {
    pub token: ShareToken,
    pub owner: UserId,
    pub conversation: ConversationId,
    pub target: ShareTarget,
    /// The conversation's title when the snapshot was taken; none while it
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

    /// Records that the share's snapshot was taken again, with the title the
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

/// Takes the conversation's snapshot under `token` (`sharing.md` § What a
/// share holds), in place of one it held: every agent's transcript as it is,
/// the root's and each subagent's, without the `context` blocks, the hidden
/// inputs that carry the execution context, instructions and skills, and
/// the `response` blocks, the usage of a request. The page hides the rest as
/// the conversation's page does.
pub fn take_conversation(transaction: &Transaction<'_>, token: &ShareToken) -> Result<(), StorageError> {
    remove(transaction, token)?;
    let agents: Vec<(i64, i64, String)> = {
        let mut statement =
            transaction.prepare("SELECT number, block_count, id FROM nodes ORDER BY number")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (agent, block_count, node) in agents {
        let node = decode("nodes", "id", demi_shared_types::NodeId::try_from(node))?;
        let blocks = tree::blocks_of(transaction, &node, block_count)?;
        let kept: Vec<&Block> = blocks
            .iter()
            .filter(|block| !matches!(block, Block::Context(_) | Block::Response(_)))
            .collect();
        keep_attachments(transaction, token, kept.iter().copied())?;
        insert(transaction, token, agent, kept)?;
    }
    // Each subagent with the description and profile it has now.
    transaction.execute(
        "INSERT INTO share_agents (token, number, description, profile)
         SELECT ?1, number, description, profile FROM nodes WHERE parent_id IS NOT NULL",
        [token.as_str()],
    )?;
    Ok(())
}

/// Takes the snapshot of the root's answer `block` under `token`: the
/// user's message that starts its request and the text. False, and nothing
/// taken, when `block` is not a complete text of the root's: the page offers
/// Share only on the text that ends a reply, and which text does is the
/// page's to judge (`web-api.md` § Sharing).
pub fn take_answer(
    transaction: &Transaction<'_>,
    token: &ShareToken,
    block: &BlockId,
) -> Result<bool, StorageError> {
    let blocks = tree::root_blocks(transaction)?;
    let Some(index) = blocks.iter().position(|held| held.id() == block) else {
        return Ok(false);
    };
    if !matches!(&blocks[index], Block::Text(text) if text.forkable) {
        return Ok(false);
    }
    let Some(message) = blocks[..index].iter().rev().find(|held| matches!(held, Block::User(_))) else {
        return Ok(false);
    };
    remove(transaction, token)?;
    keep_attachments(transaction, token, [&blocks[index]])?;
    insert(transaction, token, 0, [message, &blocks[index]])?;
    Ok(true)
}

fn insert<'a>(
    transaction: &Transaction<'_>,
    token: &ShareToken,
    agent: i64,
    blocks: impl IntoIterator<Item = &'a Block>,
) -> Result<(), StorageError> {
    let mut statement = transaction
        .prepare_cached("INSERT INTO share_blocks (token, agent, idx, block) VALUES (?1, ?2, ?3, ?4)")?;
    for (index, block) in blocks.into_iter().enumerate() {
        // The vendor's record of the session never leaves the backend, and
        // a snapshot has no session to resume.
        let mut block = block.clone();
        if let Some(entries) = block.entries_mut() {
            entries.clear();
        }
        statement.execute(params![
            token.as_str(),
            agent,
            super::columns::count(index),
            to_json(&block)
        ])?;
    }
    Ok(())
}

/// Copies into the snapshot of `token` each attachment the texts among
/// `blocks` name that exists now, so one uploaded later stays out until the
/// snapshot is taken again; its blob stays shared.
fn keep_attachments<'a>(
    transaction: &Transaction<'_>,
    token: &ShareToken,
    blocks: impl IntoIterator<Item = &'a Block>,
) -> Result<(), StorageError> {
    let mut copy = transaction.prepare_cached(
        "INSERT INTO share_attachments (token, number, name, media_type, size, width, height, blob)
         SELECT ?1, number, name, media_type, size, width, height, blob FROM attachments WHERE number = ?2
         ON CONFLICT (token, number) DO NOTHING",
    )?;
    for block in blocks {
        let Block::Text(text) = block else {
            continue;
        };
        for number in named_attachments(&text.text) {
            let number = i64::try_from(number.0).expect("a parsed attachment number fits the column");
            copy.execute(params![token.as_str(), number])?;
        }
    }
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

/// Removes the snapshot of `token`.
pub fn remove(connection: &Connection, token: &ShareToken) -> Result<(), StorageError> {
    for table in ["share_blocks", "share_agents", "share_attachments"] {
        connection.execute(&format!("DELETE FROM {table} WHERE token = ?1"), [token.as_str()])?;
    }
    Ok(())
}

/// Whether the snapshot of `token` shows `blob` (`sharing.md` § Safety): one
/// of its blocks names it, as a medium or a side of an edit, or it is the
/// blob of an attachment the snapshot keeps.
pub fn shows_blob(connection: &Connection, token: &ShareToken, blob: &BlobRef) -> Result<bool, StorageError> {
    // A block that names the blob holds its name; the rows that do not are
    // left unread.
    let mut statement = connection.prepare(
        "SELECT block FROM share_blocks WHERE token = ?1 AND instr(block, ?2) > 0",
    )?;
    let mut rows = statement.query(params![token.as_str(), blob.as_str()])?;
    while let Some(row) = rows.next()? {
        let block: Block = json("share_blocks", "block", &row.get::<_, String>(0)?)?;
        if block_blobs(&block).any(|named| named == blob) {
            return Ok(true);
        }
    }
    let attachment: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM share_attachments WHERE token = ?1 AND blob = ?2 LIMIT 1",
            params![token.as_str(), blob.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    Ok(attachment.is_some())
}

/// The attachment `number` as the snapshot of `token` keeps it.
pub fn shown_attachment(
    connection: &Connection,
    token: &ShareToken,
    number: AttachmentNumber,
) -> Result<Option<AttachmentRow>, StorageError> {
    conversation_attachments::read_shared(connection, token.as_str(), number)
}

/// A subagent the snapshot of `token` holds: its number, its spawn's
/// description and its profile, as they were when it was taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedAgent {
    pub number: u64,
    pub description: String,
    pub profile: Option<String>,
}

/// The subagents the snapshot of `token` holds, in the order they started.
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

/// Every blob the snapshots in a conversation's database name, which keeps
/// them while the snapshots do (`storage.md` § Retention).
pub fn snapshot_blobs(connection: &Connection) -> Result<BTreeSet<BlobRef>, StorageError> {
    let mut blobs = BTreeSet::new();
    let mut statement = connection.prepare("SELECT block FROM share_blocks")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let block: Block = json("share_blocks", "block", &row.get::<_, String>(0)?)?;
        blobs.extend(block_blobs(&block).cloned());
    }
    let mut statement = connection.prepare("SELECT blob FROM share_attachments")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        blobs.insert(decode("share_attachments", "blob", BlobRef::try_from(row.get::<_, String>(0)?))?);
    }
    Ok(blobs)
}
