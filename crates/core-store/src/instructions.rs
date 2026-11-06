//! The principal's personal instructions: one text, or none. A write takes a
//! text its caller checked.

use rusqlite::{OptionalExtension, params};

use super::StorageError;
use super::store::CoreStore;

impl CoreStore {
    /// The personal instructions; empty when there are none.
    pub async fn instructions(&self) -> Result<String, StorageError> {
        self.call(move |connection, _| {
            let text: Option<String> = connection
                .query_row("SELECT text FROM user_instructions WHERE id = 1", [], |row| row.get(0))
                .optional()?;
            Ok(text.unwrap_or_default())
        })
        .await
    }

    /// Replaces the personal instructions with `text`; an empty text removes
    /// them.
    pub async fn set_instructions(&self, text: String) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            if text.is_empty() {
                connection.execute("DELETE FROM user_instructions WHERE id = 1", [])?;
            } else {
                connection.execute(
                    "INSERT INTO user_instructions (id, text) VALUES (1, ?1)
                     ON CONFLICT (id) DO UPDATE SET text = excluded.text",
                    params![text],
                )?;
            }
            Ok(())
        })
        .await
    }
}
