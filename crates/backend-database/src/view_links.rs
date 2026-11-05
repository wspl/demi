//! Conversations' view links: a tab of the conversation's browser, or the
//! conversation itself.
//!
//! A link is kept by the SHA-256 of its token, never the token, and goes
//! with its conversation. One past its expiry opens nothing and lists
//! nowhere; it is deleted with the next link made in its conversation. One
//! with no expiry opens until it is deleted.

use demi_shared_types::Timestamp;
use demi_web_api_protocol::ids::ConversationId;
use rusqlite::{Row, params};

use super::StorageError;
use super::accounts::TokenHash;
use super::columns::{decode, instant, optional_instant};
use super::control::ControlService;

const TABLE: &str = "view_links";
const COLUMNS: &str = "id, conversation_id, tab, read_only, created_at, expires_at";
/// Whether a row is open at `?2`.
const OPEN: &str = "(expires_at IS NULL OR expires_at > ?2)";

/// A view link, without its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewLinkRow {
    pub id: String,
    pub conversation: ConversationId,
    /// The tab of the conversation's browser it shows, as the browser names
    /// it; none for the conversation itself.
    pub tab: Option<String>,
    /// The viewer only watches; always so for the conversation itself.
    pub read_only: bool,
    pub created_at: Timestamp,
    /// None for a link that never expires.
    pub expires_at: Option<Timestamp>,
}

impl ControlService {
    /// Stores `link` under `token`, once the conversation's links that
    /// expired by its creation are deleted; false when the conversation is
    /// gone.
    pub async fn insert_view_link(&self, token: TokenHash, link: ViewLinkRow) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "DELETE FROM view_links WHERE conversation_id = ?1 AND expires_at <= ?2",
                params![link.conversation.as_str(), link.created_at.as_millisecond()],
            )?;
            let inserted = transaction.execute(
                "INSERT INTO view_links (token_hash, id, conversation_id, tab, read_only, created_at, expires_at)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7 WHERE EXISTS (SELECT 1 FROM conversations WHERE id = ?3)",
                params![
                    token.as_str(),
                    link.id,
                    link.conversation.as_str(),
                    link.tab,
                    link.read_only,
                    link.created_at.as_millisecond(),
                    link.expires_at.map(Timestamp::as_millisecond),
                ],
            )?;
            transaction.commit()?;
            Ok(inserted == 1)
        })
        .await
    }

    /// The conversation's links that are open at `now`, of its browser's
    /// tabs or of the conversation itself as `tabs` says, newest first.
    pub async fn view_links(
        &self,
        conversation: ConversationId,
        tabs: bool,
        now: Timestamp,
    ) -> Result<Vec<ViewLinkRow>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {COLUMNS} FROM view_links WHERE conversation_id = ?1 AND {OPEN} AND (tab IS NOT NULL) = ?3
                 ORDER BY created_at DESC, id"
            ))?;
            let mut rows = statement.query(params![conversation.as_str(), now.as_millisecond(), tabs])?;
            let mut links = Vec::new();
            while let Some(row) = rows.next()? {
                links.push(link_row(row)?);
            }
            Ok(links)
        })
        .await
    }

    /// The link whose token's SHA-256 is `token`; none for an unknown one
    /// and for one that has expired at `now`.
    pub async fn view_link(&self, token: TokenHash, now: Timestamp) -> Result<Option<ViewLinkRow>, StorageError> {
        self.call(move |connection, _| {
            let mut statement =
                connection.prepare_cached(&format!("SELECT {COLUMNS} FROM view_links WHERE token_hash = ?1 AND {OPEN}"))?;
            let mut rows = statement.query(params![token.as_str(), now.as_millisecond()])?;
            rows.next()?.map(link_row).transpose()
        })
        .await
    }

    /// Deletes the conversation's link `id`, of its browser's tabs or of the
    /// conversation itself as `tabs` says; false when it has none.
    pub async fn delete_view_link(&self, conversation: ConversationId, tabs: bool, id: String) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let deleted = connection.execute(
                "DELETE FROM view_links WHERE conversation_id = ?1 AND id = ?2 AND (tab IS NOT NULL) = ?3",
                params![conversation.as_str(), id, tabs],
            )?;
            Ok(deleted == 1)
        })
        .await
    }
}

fn link_row(row: &Row<'_>) -> Result<ViewLinkRow, StorageError> {
    Ok(ViewLinkRow {
        id: row.get("id")?,
        conversation: decode(
            TABLE,
            "conversation_id",
            ConversationId::try_from(row.get::<_, String>("conversation_id")?),
        )?,
        tab: row.get("tab")?,
        read_only: row.get("read_only")?,
        created_at: instant(row, TABLE, "created_at")?,
        expires_at: optional_instant(row, TABLE, "expires_at")?,
    })
}
