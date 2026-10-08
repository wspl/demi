//! The users' personal instructions (`storage.md` § Control records;
//! `instructions.md` § Personal instructions): one text per user who has
//! any. A write takes a text its caller checked.

use demi_web_api_protocol::ids::UserId;
use rusqlite::{OptionalExtension, params};

use super::StorageError;
use super::control::ControlService;

impl ControlService {
    /// `user`'s personal instructions; empty for a user who has none.
    pub async fn instructions(&self, user: UserId) -> Result<String, StorageError> {
        self.call(move |connection, _| {
            let text: Option<String> = connection
                .query_row(
                    "SELECT text FROM user_instructions WHERE user_id = ?1",
                    [user.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(text.unwrap_or_default())
        })
        .await
    }

    /// Replaces `user`'s personal instructions with `text`; an empty text
    /// removes them.
    pub async fn set_instructions(&self, user: UserId, text: String) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            if text.is_empty() {
                connection.execute(
                    "DELETE FROM user_instructions WHERE user_id = ?1",
                    [user.as_str()],
                )?;
            } else {
                connection.execute(
                    "INSERT INTO user_instructions (user_id, text) VALUES (?1, ?2)
                     ON CONFLICT (user_id) DO UPDATE SET text = excluded.text",
                    params![user.as_str(), text],
                )?;
            }
            Ok(())
        })
        .await
    }
}
