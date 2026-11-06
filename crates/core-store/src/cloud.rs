//! What the conversations' records say of the Cloud: which of them use it, and
//! the reset each was last told of.

use demi_shared_types::ModelSelection;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, OperationId, ProviderId};
use rusqlite::{OptionalExtension, params};

use super::StorageError;
use super::columns::{decode, json};
use super::store::CoreStore;

/// One conversation as the Cloud's lifecycle weighs it
/// (`crates/backend-cloud/src/uses.rs`): whether its files and commands are on
/// the Cloud, the provider entry it infers with, and whether the Cloud is
/// attached to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudUseRecord {
    pub id: ConversationId,
    pub on_cloud: bool,
    pub provider: Option<ProviderId>,
    pub attached: bool,
}

impl CoreStore {
    /// Tells every conversation that the Cloud was reset by `operation`: each
    /// one's execution context advances once per reset, so every node reads
    /// the reset in its next context block.
    pub async fn announce_cloud_reset(&self, operation: OperationId) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "UPDATE conversations SET context_version = context_version + 1, cloud_reset_id = ?1
                 WHERE cloud_reset_id IS NULL OR cloud_reset_id <> ?1",
                [operation.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// The Cloud reset the conversation was last told of.
    pub async fn announced_cloud_reset(&self, id: ConversationId) -> Result<Option<OperationId>, StorageError> {
        self.call(move |connection, _| {
            let reset: Option<Option<String>> = connection
                .query_row(
                    "SELECT cloud_reset_id FROM conversations WHERE id = ?1",
                    [id.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            reset
                .flatten()
                .map(|reset| decode("conversations", "cloud_reset_id", OperationId::try_from(reset)))
                .transpose()
        })
        .await
    }

    /// The conversations that are not archived, as the Cloud's lifecycle
    /// weighs them; `cloud` is the Cloud device, once its first use made it.
    pub async fn cloud_uses(&self, cloud: Option<DeviceId>) -> Result<Vec<CloudUseRecord>, StorageError> {
        const TABLE: &str = "conversations";
        self.call(move |connection, _| {
            let cloud = cloud.as_ref().map(DeviceId::as_str);
            let mut statement = connection.prepare_cached(
                "SELECT c.id, c.target_kind, c.target_device_id, w.device_id AS project_device, c.model,
                        EXISTS (SELECT 1 FROM conversation_hosts h
                                WHERE h.conversation_id = c.id AND h.device_id = ?1) AS attached
                 FROM conversations c LEFT JOIN projects w ON w.id = c.target_project_id
                 WHERE c.archived_at IS NULL
                 ORDER BY c.id",
            )?;
            let mut rows = statement.query(params![cloud])?;
            let mut uses = Vec::new();
            while let Some(row) = rows.next()? {
                let kind: String = row.get("target_kind")?;
                let device: Option<String> = row.get("target_device_id")?;
                let project_device: Option<String> = row.get("project_device")?;
                let target_device = device.or(project_device);
                let on_cloud = kind == "cloud" || (cloud.is_some() && target_device.as_deref() == cloud);
                let provider = row
                    .get::<_, Option<String>>("model")?
                    .map(|model| {
                        let selection: ModelSelection = json(TABLE, "model", &model)?;
                        decode(TABLE, "model", ProviderId::try_from(selection.provider_id))
                    })
                    .transpose()?;
                uses.push(CloudUseRecord {
                    id: decode(TABLE, "id", ConversationId::try_from(row.get::<_, String>("id")?))?,
                    on_cloud,
                    provider,
                    attached: row.get("attached")?,
                });
            }
            Ok(uses)
        })
        .await
    }
}
