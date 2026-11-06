//! Shares' records.
//!
//! Each share's record is in the control database, which the token finds.
//! Its list is in its conversation's database
//! (`crates/core-store/src/shares.rs`). A record goes with its conversation;
//! the list goes with the conversation's database.

use demi_web_api_protocol::ids::{ConversationId, ShareToken, UserId};
use demi_web_api_protocol::shares::ShareTarget;
use demi_shared_types::{BlockId, Timestamp};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{decode, instant};
use super::control::ControlService;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareRecord {
    pub token: ShareToken,
    pub owner: UserId,
    pub conversation: ConversationId,
    pub target: ShareTarget,
    /// The conversation's title when the list was made; none while it
    /// was the placeholder. A reply block's share holds its target's title.
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

const SHARE_COLUMNS: &str =
    "token, user_id, conversation_id, kind, block_id, reply_block_tag, reply_block_id, title, taken_at";

fn share_row(row: &Row<'_>) -> Result<ShareRecord, StorageError> {
    let kind: String = row.get("kind")?;
    let block: Option<String> = row.get("block_id")?;
    let reply_block: (Option<String>, Option<String>) = (row.get("reply_block_tag")?, row.get("reply_block_id")?);
    let title: Option<String> = row.get("title")?;
    let target = match (kind.as_str(), block, reply_block) {
        ("conversation", None, (None, None)) => ShareTarget::Conversation,
        ("answer", Some(block), (None, None)) => ShareTarget::Answer {
            block_id: decode("shares", "block_id", BlockId::try_from(block))?,
        },
        ("reply_block", None, (Some(tag), Some(id))) => ShareTarget::ReplyBlock {
            tag,
            id,
            title: title.clone().ok_or_else(|| StorageError::Corrupt {
                table: "shares",
                column: "title",
                reason: "a reply block's share without its title".to_owned(),
            })?,
        },
        (kind, _, _) => {
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
        title,
        taken_at: instant(row, "shares", "taken_at")?,
    })
}

/// The columns that say which thing `target` is: its kind, the answer's
/// block, and the reply block's tag and ID.
struct TargetColumns<'a> {
    kind: &'static str,
    block: Option<&'a str>,
    reply_block_tag: Option<&'a str>,
    reply_block_id: Option<&'a str>,
}

fn target_columns(target: &ShareTarget) -> TargetColumns<'_> {
    let none = TargetColumns { kind: "conversation", block: None, reply_block_tag: None, reply_block_id: None };
    match target {
        ShareTarget::Conversation => none,
        ShareTarget::Answer { block_id } => TargetColumns { kind: "answer", block: Some(block_id.as_str()), ..none },
        ShareTarget::ReplyBlock { tag, id, .. } => TargetColumns {
            kind: "reply_block",
            reply_block_tag: Some(tag),
            reply_block_id: Some(id),
            ..none
        },
    }
}

fn share_of(
    connection: &Connection,
    conversation: &ConversationId,
    target: &ShareTarget,
) -> Result<Option<ShareRecord>, StorageError> {
    let columns = target_columns(target);
    connection
        .query_row(
            &format!(
                "SELECT {SHARE_COLUMNS} FROM shares
                 WHERE conversation_id = ?1 AND kind = ?2 AND block_id IS ?3
                   AND reply_block_tag IS ?4 AND reply_block_id IS ?5"
            ),
            params![
                conversation.as_str(),
                columns.kind,
                columns.block,
                columns.reply_block_tag,
                columns.reply_block_id,
            ],
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

    /// The conversation's shares, its own first, then its answers' and its
    /// reply blocks' in the order they were taken.
    pub async fn shares_of(&self, conversation: ConversationId) -> Result<Vec<ShareRecord>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare(&format!(
                "SELECT {SHARE_COLUMNS} FROM shares WHERE conversation_id = ?1
                 ORDER BY kind <> 'conversation', taken_at, token"
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
            let columns = target_columns(&share.target);
            let added = transaction.execute(
                "INSERT INTO shares
                   (token, user_id, conversation_id, kind, block_id, reply_block_tag, reply_block_id, title, taken_at)
                 SELECT ?1, ?2, id, ?3, ?4, ?8, ?9, ?5, ?6 FROM conversations WHERE id = ?7 AND user_id = ?2",
                params![
                    share.token.as_str(),
                    share.owner.as_str(),
                    columns.kind,
                    columns.block,
                    share.title,
                    share.taken_at.as_millisecond(),
                    share.conversation.as_str(),
                    columns.reply_block_tag,
                    columns.reply_block_id,
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

