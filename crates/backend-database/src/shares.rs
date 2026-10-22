//! Shares.
//!
//! Each share's record is in the control database, which the token finds.
//!
//! Its list is in its conversation's database: the messages and texts it shows by
//! the transcript rows that hold them or by its copies of the few still being
//! written, and the counts of the steps between them. A share's pages read the
//! list as a conversation's pages read its transcript
//! ([`tree::Transcript::Shared`]).
//!
//! A record goes with its conversation; the list goes with the conversation's
//! database.

use std::collections::BTreeSet;
use std::ops::Range;

use demi_agent_store::media::block_blobs;
use demi_shared_types::{BlobRef, Block, BlockId, NodeId, PictureCopy, TextBlock, Timestamp};
use demi_web_api_protocol::ids::{ConversationId, ShareToken, UserId};
use demi_web_api_protocol::shares::ShareTarget;
use comrak::nodes::NodeValue;
use comrak::{Arena, Options, parse_document};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use super::StorageError;
use super::columns::{decode, instant, json};
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

/// Makes the list of the conversation's share `token`, in place of one it had:
/// what the root's current path says, its messages and texts, with the steps
/// between them.
pub fn take_conversation(transaction: &Transaction<'_>, token: &ShareToken) -> Result<(), StorageError> {
    remove(transaction, token)?;
    let Some(root) = root(transaction)? else {
        return Ok(());
    };
    list(transaction, token, &root, 0..i64::MAX)
}

/// Makes the list of the root's answer `block` under `token`: the user's message
/// that starts its request, through the text, with the texts and the steps between
/// them, along the root's current path.
///
/// False, and nothing made, when `block` is not a complete text of that path: the
/// page offers Share only on the text that ends a reply, and which text does is
/// the page's to judge.
pub fn take_answer(
    transaction: &Transaction<'_>,
    token: &ShareToken,
    block: &BlockId,
) -> Result<bool, StorageError> {
    let Some(root) = root(transaction)? else {
        return Ok(false);
    };
    let found: Option<(i64, String)> = transaction
        .query_row(
            "SELECT blocks.idx, blocks.block FROM blocks INDEXED BY blocks_by_id CROSS JOIN path_runs AS run
               ON run.node_id = blocks.node_id AND blocks.idx >= run.first AND blocks.idx < run.first + run.length
             WHERE json_extract(blocks.block, '$.id') = ?1 AND blocks.node_id = ?2",
            params![block.as_str(), root],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((index, answer)) = found else {
        return Ok(false);
    };
    let Block::Text(text) = json("blocks", "block", &answer)? else {
        return Ok(false);
    };
    if !text.forkable {
        return Ok(false);
    }
    // Along a path, a row's number is above the one it follows, so the
    // message is the path's latest user row before the answer's.
    let message: Option<i64> = transaction
        .query_row(
            "SELECT blocks.idx FROM path_runs AS run CROSS JOIN blocks INDEXED BY blocks_by_type
               ON blocks.node_id = run.node_id AND json_extract(blocks.block, '$.type') = 'user'
                 AND blocks.idx >= run.first AND blocks.idx < run.first + run.length
             WHERE run.node_id = ?1 AND blocks.idx < ?2
             ORDER BY blocks.idx DESC LIMIT 1",
            params![root, index],
            |row| row.get(0),
        )
        .optional()?;
    let Some(message) = message else {
        return Ok(false);
    };
    remove(transaction, token)?;
    list(transaction, token, &root, message..index + 1)?;
    Ok(true)
}

/// The root's node id; none in a conversation without one.
fn root(transaction: &Transaction<'_>) -> Result<Option<String>, StorageError> {
    Ok(transaction
        .query_row("SELECT id FROM nodes WHERE parent_id IS NULL", [], |row| row.get(0))
        .optional()?)
}

/// The block types a share shows, the root's messages and texts.
const SAID: &str = "('user', 'steer', 'text')";

/// The block types a share counts as steps not shared: thinking and calls, a
/// command's or a subagent's start among them. Every other block shows nothing
/// and counts nothing.
const STEPS: &str = "('thinking', 'redacted_thinking', 'tool_call')";

/// Lists under `token` the rows of the root's current path numbered within `rows`,
/// the one place a share's list is made.
///
/// The list holds each message and text by the row that holds it, in order, and in
/// place of each run of steps around them one entry of their count. Along a path,
/// each row's number is above the one it follows, so the rows' order is the path's.
///
/// The list is made from the transcript's index of block types, without reading a
/// block: only the texts still being written are read, to copy them, and the
/// copies, for the blobs they name.
///
/// A text still being written is one after the last `response` of the root's path,
/// which ends each request the model answered: the model may still be writing it.
/// Its entry points at its copy, as it stands, without the vendor's record of the
/// session, which never leaves the backend.
fn list(transaction: &Transaction<'_>, token: &ShareToken, root: &str, rows: Range<i64>) -> Result<(), StorageError> {
    // Each step is counted in the run of those after the same number of
    // messages and texts; a run is placed where its first step stands.
    transaction.execute(
        &format!(
            "WITH listed AS (
               SELECT blocks.idx, json_extract(blocks.block, '$.type') IN {SAID} AS said
               FROM path_runs AS run CROSS JOIN blocks INDEXED BY blocks_by_type
                 ON blocks.node_id = run.node_id AND blocks.idx >= run.first AND blocks.idx < run.first + run.length
                   AND (json_extract(blocks.block, '$.type') IN {SAID} OR json_extract(blocks.block, '$.type') IN {STEPS})
               WHERE run.node_id = ?2 AND blocks.idx >= ?3 AND blocks.idx < ?4
             ),
             runs AS (SELECT idx, said, sum(said) OVER (ORDER BY idx) AS before FROM listed),
             entries AS (
               SELECT idx AS at, idx, NULL AS steps FROM runs WHERE said
               UNION ALL
               SELECT min(idx), NULL, count(*) FROM runs WHERE NOT said GROUP BY before
             )
             INSERT INTO share_entries (token, position, node_id, idx, steps)
             SELECT ?1, row_number() OVER (ORDER BY at) - 1, iif(idx IS NULL, NULL, ?2), idx, steps FROM entries"
        ),
        params![token.as_str(), root, rows.start, rows.end],
    )?;
    transaction.execute(
        "INSERT INTO share_blocks (token, idx, block)
         SELECT entry.token, entry.position, json_remove(row.block, '$.entries')
         FROM share_entries AS entry JOIN blocks AS row ON row.node_id = entry.node_id AND row.idx = entry.idx
         WHERE entry.token = ?1 AND json_extract(row.block, '$.type') = 'text'
           AND entry.idx > coalesce(
             (SELECT blocks.idx FROM path_runs AS run CROSS JOIN blocks INDEXED BY blocks_by_type
                ON blocks.node_id = run.node_id AND json_extract(blocks.block, '$.type') = 'response'
                  AND blocks.idx >= run.first AND blocks.idx < run.first + run.length
              WHERE run.node_id = ?2 ORDER BY blocks.idx DESC LIMIT 1),
             -1)",
        params![token.as_str(), root],
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
    name_copied_blobs(transaction, token)
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

/// Keeps what every share shows of the row `index` of `node` before a write
/// replaces it, as a later event marks it.
///
/// A listed row is copied into the share as it stands, with the blobs it names,
/// and its entry points at the copy, in the write's transaction. The one guard of
/// every write to `blocks` that changes a row; no write removes one.
pub(crate) fn keep_rewritten(transaction: &Transaction<'_>, node: &NodeId, index: i64) -> Result<(), StorageError> {
    let rows = "entry.node_id = ?1 AND entry.idx = ?2";
    let parameters = params![node.as_str(), index];
    transaction
        .prepare_cached(&format!(
            "INSERT INTO share_blocks (token, idx, block)
             SELECT entry.token, entry.position, json_remove(row.block, '$.entries')
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
         WHERE share_entries.token = ?1 AND copy.token = ?1 AND copy.idx = share_entries.position",
        [token.as_str()],
    )?;
    Ok(())
}

/// Copies into the share whose token is `?1` of `parameters` each attachment the
/// query `named` lists, by its `attachment` number, that exists now, whoever
/// stored it.
///
/// One uploaded later stays out until the list is made again. Its blob stays
/// shared.
fn keep_attachments(
    transaction: &Transaction<'_>,
    named: &str,
    parameters: &[&dyn rusqlite::ToSql],
) -> Result<(), StorageError> {
    transaction.execute(
        &format!(
            "INSERT OR IGNORE INTO share_attachments (token, number, name, media_type, size, width, height, blob)
             SELECT ?1, number, name, media_type, size, width, height, blob
             FROM attachments AS attachment JOIN ({named}) AS named ON named.attachment = attachment.number"
        ),
        parameters,
    )?;
    Ok(())
}

/// The attachments a text delivers as blocks.
///
/// A text delivers an attachment in each line whose whole content, after the
/// Markdown that places it (a list item's marker, a quote's `>`), is the tag
/// `<attachment id="a3" />`, the space before `/>` optional. A tag in code, in an
/// HTML comment or block, or among other content on its line names none, nor does
/// a link or an image to `attachment:a3`.
///
/// The web app's renderer follows the same rule
/// (`packages/web-ui/src/markdown/attachment-block.ts`); both are held to the
/// cases of
/// `crates/shared-types/tests/shared-types/fixtures/attachment-blocks.json`.
///
/// CommonMark reads the tag as raw HTML, which starts an HTML block that runs on
/// to the next blank line, while the renderer gives the tag's line a block of its
/// own. So each tag is read as a word of its own first, a mark no text holds, and
/// a block is a paragraph's line that holds only that word: code, an HTML block or
/// comment keep it as their text, emphasis wraps it, other words join it.
pub fn named_attachments(text: &str) -> BTreeSet<AttachmentNumber> {
    if !text.contains("<attachment") {
        return BTreeSet::new();
    }
    let marked = mark_tags(text);
    let arena = Arena::new();
    // The page's renderer reads GitHub's tables, strikethrough and autolinks
    // and `$`-math too, each of which can take a tag's line as its text; the
    // same extensions here keep the two on one reading.
    let mut options = Options::default();
    options.extension.table = true;
    options.extension.strikethrough = true;
    options.extension.autolink = true;
    options.extension.math_dollars = true;
    let root = parse_document(&arena, &marked, &options);
    let alone = |node: Option<&comrak::nodes::AstNode<'_>>| {
        node.is_none_or(|node| matches!(node.data.borrow().value, NodeValue::SoftBreak | NodeValue::LineBreak))
    };
    root.descendants()
        .filter_map(|node| {
            let data = node.data.borrow();
            let NodeValue::Text(word) = &data.value else {
                return None;
            };
            let number = word.strip_prefix(TAG_MARK)?.strip_suffix(TAG_MARK)?.parse().ok()?;
            let line = node.parent()?;
            let line_data = line.data.borrow();
            let placed = match &line_data.value {
                NodeValue::Paragraph => true,
                // The line above a rule, as in `<attachment id="a1" />` then `---`.
                NodeValue::Heading(heading) => heading.setext,
                _ => false,
            };
            // A line indented four columns past its paragraph is the
            // paragraph's text, as it would be indented code at a block's start.
            let indented = data.sourcepos.start.column >= line_data.sourcepos.start.column + 4;
            (placed && !indented && alone(node.previous_sibling()) && alone(node.next_sibling())).then_some(number)
        })
        .collect()
}

/// The attachments a text block names: those its text delivers
/// ([`named_attachments`]) and the copies it keeps of the pictures of the Host
/// it shows.
pub fn block_attachments(text: &TextBlock) -> BTreeSet<AttachmentNumber> {
    let mut named = named_attachments(&text.text);
    for picture in text.pictures.iter().flatten() {
        if let PictureCopy::Copied { attachment } = &picture.copy
            && let Ok(number) = attachment.parse()
        {
            named.insert(number);
        }
    }
    named
}

/// What stands for a tag in the text CommonMark reads: a private-use
/// character, which no Markdown syntax gives a meaning, on each side of the
/// number.
const TAG_MARK: char = '\u{E000}';

/// `text` with each `<attachment id="a3" />` as the number between two
/// [`TAG_MARK`]s.
fn mark_tags(text: &str) -> String {
    let mut marked = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<attachment id=\"") {
        marked.push_str(&rest[..start]);
        let after = &rest[start + "<attachment id=\"".len()..];
        let tag = after.split_once('"').and_then(|(id, tail)| {
            let tail = tail.strip_prefix(' ').unwrap_or(tail).strip_prefix("/>")?;
            let digits = id.strip_prefix('a')?;
            let number = !digits.is_empty() && !digits.starts_with('0') && digits.bytes().all(|byte| byte.is_ascii_digit());
            number.then_some((id, tail))
        });
        match tag {
            Some((id, tail)) => {
                marked.extend([TAG_MARK]);
                marked.push_str(id);
                marked.extend([TAG_MARK]);
                rest = tail;
            }
            None => {
                marked.push('<');
                rest = &rest[start + 1..];
            }
        }
    }
    marked.push_str(rest);
    marked
}

/// What a share's list holds, by count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShareCounts {
    /// The list's blocks.
    pub blocks: u64,
    /// The transcripts they come from: the root's and its subagents'.
    pub nodes: u64,
    /// The attachments the share keeps.
    pub attachments: u64,
}

/// What the share `token`'s list holds now.
pub fn counts(connection: &Connection, token: &ShareToken) -> Result<ShareCounts, StorageError> {
    let count = |sql: &str| -> Result<u64, StorageError> {
        let counted: i64 = connection.query_row(sql, [token.as_str()], |row| row.get(0))?;
        Ok(u64::try_from(counted).unwrap_or(0))
    };
    Ok(ShareCounts {
        blocks: count("SELECT COUNT(*) FROM share_entries WHERE token = ?1")?,
        nodes: count("SELECT COUNT(DISTINCT node_id) FROM share_entries WHERE token = ?1")?,
        attachments: count("SELECT COUNT(*) FROM share_attachments WHERE token = ?1")?,
    })
}

/// Removes the share `token`'s list and copies.
pub fn remove(connection: &Connection, token: &ShareToken) -> Result<(), StorageError> {
    for table in ["share_entries", "share_blocks", "share_blobs", "share_attachments"] {
        connection.execute(&format!("DELETE FROM {table} WHERE token = ?1"), [token.as_str()])?;
    }
    Ok(())
}

/// Whether the share `token` shows `blob`: a block on its list names it, as a
/// medium or a side of an edit, by `block_blobs` or by its copies'
/// `share_blobs`, or it is the blob of an attachment the share keeps. No block
/// is read.
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

/// Every blob the shares' copies and kept attachments in a conversation's
/// database name, which keeps them while the shares do; the other blocks a
/// list shows are the transcript's, which keeps theirs.
pub fn copied_blobs(connection: &Connection) -> Result<BTreeSet<BlobRef>, StorageError> {
    let mut blobs = BTreeSet::new();
    let mut statement = connection.prepare("SELECT blob FROM share_blobs UNION SELECT blob FROM share_attachments")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        blobs.insert(decode("share_blobs", "blob", BlobRef::try_from(row.get::<_, String>(0)?))?);
    }
    Ok(blobs)
}
