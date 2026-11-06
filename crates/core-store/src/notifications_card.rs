//! The notifications card of a conversation.
//!
//! A conversation whose agent found that nothing takes its notifications shows
//! the user a card that turns them on. Its status is none, `shown` or
//! `dismissed`: the agent's check raises it from none to `shown`, a subscription
//! that takes the agent's notifications sets every `shown` back to none, and
//! Dismiss sets `dismissed`, which stays, so the conversation never shows it
//! again. Whether anything takes the notifications is its program's to say.

use demi_web_api_protocol::ids::ConversationId;
use rusqlite::{OptionalExtension, params};

use super::StorageError;
use super::columns::decode;
use super::store::CoreStore;

impl CoreStore {
    /// Shows the card in `conversation` unless it shows it or the user
    /// dismissed it there. Answers whether it now shows it newly.
    pub async fn raise_notifications_card(&self, conversation: ConversationId) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let changed = connection.execute(
                "UPDATE conversations SET notifications_card = 'shown'
                  WHERE id = ?1 AND notifications_card IS NULL",
                params![conversation.as_str()],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    /// Takes the card from every conversation that shows it. Answers those
    /// conversations.
    pub async fn clear_notifications_cards(&self) -> Result<Vec<ConversationId>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "UPDATE conversations SET notifications_card = NULL
                  WHERE notifications_card = 'shown' RETURNING id",
            )?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.map(|id| decode("conversations", "id", ConversationId::try_from(id?)))
                .collect()
        })
        .await
    }

    /// Dismisses the card in `conversation` for good. Answers whether it
    /// showed it.
    pub async fn dismiss_notifications_card(&self, conversation: ConversationId) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let changed = connection.execute(
                "UPDATE conversations SET notifications_card = 'dismissed'
                  WHERE id = ?1 AND notifications_card IS NOT 'dismissed'",
                params![conversation.as_str()],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    /// Whether `conversation` shows the card.
    pub async fn notifications_card(&self, conversation: ConversationId) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let shown = connection
                .query_row(
                    "SELECT notifications_card = 'shown' FROM conversations WHERE id = ?1",
                    params![conversation.as_str()],
                    |row| row.get::<_, Option<bool>>(0),
                )
                .optional()?;
            Ok(shown.flatten().unwrap_or(false))
        })
        .await
    }
}
