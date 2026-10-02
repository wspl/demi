//! Work panels (`storage.md` § Control records): each conversation's tabs,
//! the ids it ever had and the revision that counts its changes. A change
//! reads the row and writes it in one transaction, so one conversation's
//! changes apply one at a time. The backend never interprets a tab.

use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::panel::{Applied, PanelChange, PanelDocument, PanelEffect, WorkPanel};
use rusqlite::{Connection, OptionalExtension, params};

use super::StorageError;
use super::columns::{decode, json, to_json};
use super::control::ControlService;
use super::drafts::archived;

/// What a change came to.
#[derive(Debug, Clone, PartialEq)]
pub enum PanelOutcome {
    /// The change is in the panel, which is at `revision` now.
    Changed {
        revision: u64,
        effect: PanelEffect,
    },
    /// The change had nothing to do; the panel stays at `revision`.
    Unchanged {
        revision: u64,
    },
    /// A create past the most tabs or the most data.
    Full,
    /// An update past the most data.
    TooLarge,
    Archived,
    /// No conversation has that id.
    Missing,
}

impl ControlService {
    /// The conversation's panel; the empty one when it never changed. A
    /// stored document that no longer matches the panel's shape is corrupt,
    /// not repaired.
    pub async fn panel(&self, conversation: ConversationId) -> Result<WorkPanel, StorageError> {
        self.call(move |connection, _| {
            let (revision, document) = stored(connection, &conversation)?;
            Ok(WorkPanel {
                revision,
                tabs: document.tabs,
            })
        })
        .await
    }

    /// Applies `change` to the conversation's panel.
    pub async fn change_panel(
        &self,
        conversation: ConversationId,
        change: PanelChange,
    ) -> Result<PanelOutcome, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let exists: Option<i64> = transaction
                .query_row(
                    "SELECT 1 FROM conversations WHERE id = ?1",
                    [conversation.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            if exists.is_none() {
                return Ok(PanelOutcome::Missing);
            }
            if archived(&transaction, &conversation)? {
                return Ok(PanelOutcome::Archived);
            }
            let (revision, mut document) = stored(&transaction, &conversation)?;
            let effect = match document.apply(change) {
                Applied::Effect(effect) => effect,
                Applied::Nothing => return Ok(PanelOutcome::Unchanged { revision }),
                Applied::Full => return Ok(PanelOutcome::Full),
                Applied::TooLarge => return Ok(PanelOutcome::TooLarge),
            };
            let revision = revision + 1;
            transaction.execute(
                "INSERT INTO conversation_panels (conversation_id, revision, document, updated_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (conversation_id) DO UPDATE SET revision = excluded.revision, document = excluded.document, updated_at = excluded.updated_at",
                params![
                    conversation.as_str(),
                    i64::try_from(revision).expect("a revision fits in 63 bits"),
                    to_json(&document),
                    now.as_millisecond()
                ],
            )?;
            transaction.commit()?;
            Ok(PanelOutcome::Changed { revision, effect })
        })
        .await
    }
}

/// The conversation's revision and document; 0 and the empty one before
/// its first change.
fn stored(
    connection: &Connection,
    conversation: &ConversationId,
) -> Result<(u64, PanelDocument), StorageError> {
    let row: Option<(i64, String)> = connection
        .query_row(
            "SELECT revision, document FROM conversation_panels WHERE conversation_id = ?1",
            [conversation.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((revision, document)) = row else {
        return Ok((0, PanelDocument::default()));
    };
    let revision = decode("conversation_panels", "revision", u64::try_from(revision))?;
    let document = json("conversation_panels", "document", &document)?;
    Ok((revision, document))
}
