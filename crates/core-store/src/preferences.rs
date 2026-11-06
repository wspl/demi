//! The saved preferences: one row, none until the first is saved.

use demi_web_api_protocol::settings::Preferences;
use rusqlite::{Connection, OptionalExtension};

use super::StorageError;
use super::columns::{json, to_json};
use super::store::CoreStore;

impl CoreStore {
    /// The saved preferences; with none saved there are no overrides.
    pub async fn preferences(&self) -> Result<Preferences, StorageError> {
        self.call(move |connection, _| saved(connection)).await
    }

    /// Merges a patch into the preferences with `merge`, which takes the
    /// saved preferences and answers them patched, and answers them as saved.
    /// The read, the merge and the write are one transaction, so patches of
    /// different fields that arrive together all stay.
    pub async fn patch_preferences(
        &self,
        merge: impl FnOnce(Preferences) -> Preferences + Send + 'static,
    ) -> Result<Preferences, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let preferences = merge(saved(&transaction)?);
            transaction.execute(
                "INSERT INTO user_preferences (id, preferences) VALUES (1, ?1)
                 ON CONFLICT (id) DO UPDATE SET preferences = excluded.preferences",
                [to_json(&preferences)],
            )?;
            transaction.commit()?;
            Ok(preferences)
        })
        .await
    }
}

fn saved(connection: &Connection) -> Result<Preferences, StorageError> {
    let saved: Option<String> = connection
        .query_row("SELECT preferences FROM user_preferences", [], |row| row.get(0))
        .optional()?;
    match saved {
        Some(text) => json("user_preferences", "preferences", &text),
        None => Ok(Preferences::default()),
    }
}
