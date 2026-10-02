//! Plugin values (`storage.md` § Control records; `plugins.md` § The
//! contract): each plugin's JSON documents for a user, by key, each with
//! the revision a conditional write names. The control database decodes a
//! document only as JSON; its plugin decodes it into its own type.

use std::collections::BTreeMap;

use demi_web_api_protocol::ids::UserId;
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use super::StorageError;
use super::columns::decode;
use super::control::ControlService;

const TABLE: &str = "plugin_values";

/// A stored value and its revision.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginValue {
    pub document: Value,
    pub revision: u64,
}

impl ControlService {
    /// `plugin`'s value `key` for `user`.
    pub async fn plugin_value(
        &self,
        user: UserId,
        plugin: String,
        key: String,
    ) -> Result<Option<PluginValue>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT document, revision FROM plugin_values
                 WHERE user_id = ?1 AND plugin = ?2 AND key = ?3",
            )?;
            let row = statement
                .query_row(params![user.as_str(), plugin, key], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .optional()?;
            row.map(|(document, revision)| plugin_value(&document, revision))
                .transpose()
        })
        .await
    }

    /// Every value of `plugin`'s for `user`, by key.
    pub async fn plugin_values(
        &self,
        user: UserId,
        plugin: String,
    ) -> Result<BTreeMap<String, PluginValue>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT key, document, revision FROM plugin_values
                 WHERE user_id = ?1 AND plugin = ?2",
            )?;
            let mut rows = statement.query(params![user.as_str(), plugin])?;
            let mut values = BTreeMap::new();
            while let Some(row) = rows.next()? {
                let key: String = row.get(0)?;
                let value = plugin_value(&row.get::<_, String>(1)?, row.get(2)?)?;
                values.insert(key, value);
            }
            Ok(values)
        })
        .await
    }

    /// Writes `plugin`'s value `key` for `user` if it is still at
    /// `revision`, none for a value that does not exist yet, in one
    /// statement, so two writes never build on the same revision. Answers
    /// the new revision, or none when another write came first.
    pub async fn write_plugin_value(
        &self,
        user: UserId,
        plugin: String,
        key: String,
        document: Value,
        revision: Option<u64>,
    ) -> Result<Option<u64>, StorageError> {
        let text = document.to_string();
        self.call(move |connection, _| {
            let written = match revision {
                None => connection.execute(
                    "INSERT INTO plugin_values (user_id, plugin, key, document, revision)
                     VALUES (?1, ?2, ?3, ?4, 1) ON CONFLICT DO NOTHING",
                    params![user.as_str(), plugin, key, text],
                )?,
                Some(revision) => connection.execute(
                    "UPDATE plugin_values SET document = ?4, revision = revision + 1
                     WHERE user_id = ?1 AND plugin = ?2 AND key = ?3 AND revision = ?5",
                    params![
                        user.as_str(),
                        plugin,
                        key,
                        text,
                        i64::try_from(revision).unwrap_or(i64::MAX)
                    ],
                )?,
            };
            Ok((written == 1).then(|| revision.map_or(1, |revision| revision + 1)))
        })
        .await
    }
}

fn plugin_value(document: &str, revision: i64) -> Result<PluginValue, StorageError> {
    Ok(PluginValue {
        document: decode(TABLE, "document", serde_json::from_str(document))?,
        revision: decode(TABLE, "revision", u64::try_from(revision))?,
    })
}
