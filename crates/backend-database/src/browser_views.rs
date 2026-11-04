//! The view links of conversations' browsers.
//!
//! A view is kept by the SHA-256 of its token, never the token, and goes
//! with its conversation. One past its expiry opens nothing and lists
//! nowhere; it is deleted with the next view made in its conversation.

use demi_shared_types::Timestamp;
use demi_web_api_protocol::ids::ConversationId;
use rusqlite::{Row, params};

use super::StorageError;
use super::accounts::TokenHash;
use super::columns::{decode, instant};
use super::control::ControlService;

const COLUMNS: &str = "id, conversation_id, tab, read_only, created_at, expires_at";

/// A view of one tab of a conversation's browser, without its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserViewRow {
    pub id: String,
    pub conversation: ConversationId,
    /// The tab's id, as the browser names it.
    pub tab: String,
    pub read_only: bool,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
}

impl ControlService {
    /// Stores `view` under `token`, once the conversation's views that
    /// expired by the view's creation are deleted; false when the
    /// conversation is gone.
    pub async fn insert_browser_view(&self, token: TokenHash, view: BrowserViewRow) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "DELETE FROM browser_views WHERE conversation_id = ?1 AND expires_at <= ?2",
                params![view.conversation.as_str(), view.created_at.as_millisecond()],
            )?;
            let inserted = transaction.execute(
                "INSERT INTO browser_views (token_hash, id, conversation_id, tab, read_only, created_at, expires_at)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7 WHERE EXISTS (SELECT 1 FROM conversations WHERE id = ?3)",
                params![
                    token.as_str(),
                    view.id,
                    view.conversation.as_str(),
                    view.tab,
                    view.read_only,
                    view.created_at.as_millisecond(),
                    view.expires_at.as_millisecond(),
                ],
            )?;
            transaction.commit()?;
            Ok(inserted == 1)
        })
        .await
    }

    /// The conversation's views that have not expired at `now`, newest
    /// first.
    pub async fn browser_views(&self, conversation: ConversationId, now: Timestamp) -> Result<Vec<BrowserViewRow>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {COLUMNS} FROM browser_views WHERE conversation_id = ?1 AND expires_at > ?2
                 ORDER BY created_at DESC, id"
            ))?;
            let mut rows = statement.query(params![conversation.as_str(), now.as_millisecond()])?;
            let mut views = Vec::new();
            while let Some(row) = rows.next()? {
                views.push(view_row(row)?);
            }
            Ok(views)
        })
        .await
    }

    /// The view whose token's SHA-256 is `token`; none for an unknown one
    /// and for one that has expired at `now`.
    pub async fn browser_view(&self, token: TokenHash, now: Timestamp) -> Result<Option<BrowserViewRow>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {COLUMNS} FROM browser_views WHERE token_hash = ?1 AND expires_at > ?2"
            ))?;
            let mut rows = statement.query(params![token.as_str(), now.as_millisecond()])?;
            rows.next()?.map(view_row).transpose()
        })
        .await
    }

    /// Deletes the conversation's view `id`; false when it has none.
    pub async fn delete_browser_view(&self, conversation: ConversationId, id: String) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let deleted = connection.execute(
                "DELETE FROM browser_views WHERE conversation_id = ?1 AND id = ?2",
                params![conversation.as_str(), id],
            )?;
            Ok(deleted == 1)
        })
        .await
    }
}

fn view_row(row: &Row<'_>) -> Result<BrowserViewRow, StorageError> {
    Ok(BrowserViewRow {
        id: row.get("id")?,
        conversation: decode(
            "browser_views",
            "conversation_id",
            ConversationId::try_from(row.get::<_, String>("conversation_id")?),
        )?,
        tab: row.get("tab")?,
        read_only: row.get("read_only")?,
        created_at: instant(row, "browser_views", "created_at")?,
        expires_at: instant(row, "browser_views", "expires_at")?,
    })
}
