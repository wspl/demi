//! What a plugin's package on a Host last reported of a conversation.
//!
//! A package tells its plugin the whole of what it holds for a conversation
//! when that changes, an empty state too; the backend keeps the latest per
//! plugin, conversation and Host. A page reads it from here at once, also
//! after a restart of the backend while the Host is away, and never asks the
//! Host (`crates/plugin-interface/src/request.rs`).

use demi_shared_types::Timestamp;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use super::StorageError;
use super::columns::{decode, instant, to_json};
use super::control::ControlService;

const TABLE: &str = "plugin_reports";

/// A report as last kept.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginReport {
    /// The package's own count of its changes.
    pub revision: u64,
    pub value: Value,
    pub reported_at: Timestamp,
}

impl ControlService {
    /// What `plugin`'s package on the Host of `device` last reported of
    /// `conversation`; none before it reported.
    pub async fn plugin_report(
        &self,
        plugin: String,
        conversation: ConversationId,
        device: DeviceId,
    ) -> Result<Option<PluginReport>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT revision, value, reported_at FROM plugin_reports
                 WHERE plugin_id = ?1 AND conversation_id = ?2 AND device_id = ?3",
            )?;
            let mut rows = statement.query(params![plugin, conversation.as_str(), device.as_str()])?;
            let Some(row) = rows.next()? else {
                return Ok(None);
            };
            Ok(Some(PluginReport {
                revision: decode(TABLE, "revision", u64::try_from(row.get::<_, i64>("revision")?))?,
                value: decode(TABLE, "value", serde_json::from_str(&row.get::<_, String>("value")?))?,
                reported_at: instant(row, TABLE, "reported_at")?,
            }))
        })
        .await
    }

    /// Keeps `value` as what `plugin`'s package on the Host of `device`
    /// reported of `conversation` now; answers whether it differs from what
    /// was kept. A conversation that is gone keeps nothing and answers false.
    pub async fn keep_plugin_report(
        &self,
        plugin: String,
        conversation: ConversationId,
        device: DeviceId,
        revision: u64,
        value: Value,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, now| {
            let value = to_json(&value);
            let transaction = connection.transaction()?;
            let exists: Option<i64> = transaction
                .query_row("SELECT 1 FROM conversations WHERE id = ?1", [conversation.as_str()], |row| row.get(0))
                .optional()?;
            if exists.is_none() {
                return Ok(false);
            }
            let kept: Option<String> = transaction
                .prepare_cached(
                    "SELECT value FROM plugin_reports WHERE plugin_id = ?1 AND conversation_id = ?2 AND device_id = ?3",
                )?
                .query_row(params![plugin, conversation.as_str(), device.as_str()], |row| row.get(0))
                .optional()?;
            let changed = kept.as_ref() != Some(&value);
            transaction.execute(
                "INSERT INTO plugin_reports (plugin_id, conversation_id, device_id, revision, value, reported_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (plugin_id, conversation_id, device_id) DO UPDATE SET
                   revision = excluded.revision,
                   value = excluded.value,
                   reported_at = excluded.reported_at",
                params![
                    plugin,
                    conversation.as_str(),
                    device.as_str(),
                    i64::try_from(revision).unwrap_or(i64::MAX),
                    value,
                    now.as_millisecond()
                ],
            )?;
            transaction.commit()?;
            Ok(changed)
        })
        .await
    }
}
