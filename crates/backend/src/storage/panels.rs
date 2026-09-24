//! Work panel documents (`storage.md` § Control records): each
//! conversation's one JSON document, replaced whole by every save and
//! deleted with its conversation. The backend never interprets a tab.

use demi_web_api::ids::ConversationId;
use demi_web_api::panel::WorkPanel;
use rusqlite::{OptionalExtension, params};

use super::StorageError;
use super::columns::json;
use super::control::ControlService;

impl ControlService {
    /// The conversation's saved panel; `None` when it never saved one. A
    /// stored document that no longer matches the panel's shape is corrupt,
    /// not repaired.
    pub(crate) async fn panel(&self, conversation: ConversationId) -> Result<Option<WorkPanel>, StorageError> {
        self.call(move |connection, _| {
            let document: Option<String> = connection
                .query_row(
                    "SELECT document FROM conversation_panels WHERE conversation_id = ?1",
                    [conversation.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            document
                .map(|document| json("conversation_panels", "document", &document))
                .transpose()
        })
        .await
    }

    /// Replaces the conversation's panel with `document`, the panel's JSON.
    pub(crate) async fn save_panel(&self, conversation: ConversationId, document: String) -> Result<(), StorageError> {
        self.call(move |connection, now| {
            connection.execute(
                "INSERT INTO conversation_panels (conversation_id, document, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (conversation_id) DO UPDATE SET document = excluded.document, updated_at = excluded.updated_at",
                params![conversation.as_str(), document, now.as_millisecond()],
            )?;
            Ok(())
        })
        .await
    }
}
