//! The records of each user's Cloud (`storage.md` § Control records,
//! `managed-hosts.md` § System reset): its reset operations, the token each
//! boot mints, the announcement of a reset to the user's conversations, and
//! which of the user's conversations use the Cloud.

use demi_machines_protocol::BaseVersion;
use demi_web_api::cloud::ResetPhase;
use demi_web_api::ids::{ConversationId, DeviceId, OperationId, ProviderId, UserId};
use rusqlite::{OptionalExtension, Row, params};

use super::StorageError;
use super::columns::decode;
use super::control::ControlService;
use crate::auth::sessions::TokenHash;

/// A reset of a Cloud, as its intent is kept: the base it selected when it
/// was admitted, the phase it reached, and why it failed. A retry resumes
/// the same operation on the same base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedOperation {
    pub(crate) id: OperationId,
    pub(crate) base_version: BaseVersion,
    pub(crate) phase: ResetPhase,
    pub(crate) error: Option<String>,
}

/// One of the user's conversations as the Cloud's lifecycle weighs it
/// (`sessions-and-targets.md` § How a conversation uses a device): whether
/// its files and commands are on the Cloud, the provider entry it infers
/// with, and whether the Cloud is attached to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CloudUseRecord {
    pub(crate) id: ConversationId,
    pub(crate) on_cloud: bool,
    pub(crate) provider: Option<ProviderId>,
    pub(crate) attached: bool,
}

const TABLE: &str = "managed_operations";
const OPERATION_COLUMNS: &str = "operation_id, base_version, phase, error";

impl ControlService {
    /// The device's reset `id`, as it was last written.
    pub(crate) async fn managed_operation(
        &self,
        device: DeviceId,
        id: OperationId,
    ) -> Result<Option<ManagedOperation>, StorageError> {
        self.call(move |connection, _| {
            connection
                .query_row(
                    &format!("SELECT {OPERATION_COLUMNS} FROM {TABLE} WHERE device_id = ?1 AND operation_id = ?2"),
                    [device.as_str(), id.as_str()],
                    |row| Ok(operation_row(row)),
                )
                .optional()?
                .transpose()
        })
        .await
    }

    /// The device's reset written last: the one its status shows.
    pub(crate) async fn latest_managed_operation(&self, device: DeviceId) -> Result<Option<ManagedOperation>, StorageError> {
        self.call(move |connection, _| {
            connection
                .query_row(
                    &format!(
                        "SELECT {OPERATION_COLUMNS} FROM {TABLE} WHERE device_id = ?1
                         ORDER BY updated_at DESC, rowid DESC LIMIT 1"
                    ),
                    [device.as_str()],
                    |row| Ok(operation_row(row)),
                )
                .optional()?
                .transpose()
        })
        .await
    }

    /// Writes the operation's phase and error, creating its row on its
    /// first write.
    pub(crate) async fn put_managed_operation(
        &self,
        device: DeviceId,
        operation: ManagedOperation,
    ) -> Result<(), StorageError> {
        self.call(move |connection, now| {
            connection.execute(
                &format!(
                    "INSERT INTO {TABLE} (device_id, operation_id, base_version, phase, error, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT (device_id, operation_id) DO UPDATE
                       SET phase = excluded.phase, error = excluded.error, updated_at = excluded.updated_at"
                ),
                params![
                    device.as_str(),
                    operation.id.as_str(),
                    operation.base_version.as_str(),
                    operation.phase.to_string(),
                    operation.error,
                    now.as_millisecond()
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// Every reset left between its admission and its end, with its
    /// device: what a backend that stopped in the middle of one finishes
    /// when it starts.
    pub(crate) async fn unfinished_managed_operations(&self) -> Result<Vec<(DeviceId, ManagedOperation)>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare(&format!(
                "SELECT device_id, {OPERATION_COLUMNS} FROM {TABLE}
                 WHERE phase NOT IN ('ready', 'failed') ORDER BY updated_at, rowid"
            ))?;
            let mut rows = statement.query([])?;
            let mut unfinished = Vec::new();
            while let Some(row) = rows.next()? {
                let device = decode(TABLE, "device_id", DeviceId::try_from(row.get::<_, String>("device_id")?))?;
                unfinished.push((device, operation_row(row)?));
            }
            Ok(unfinished)
        })
        .await
    }

    /// Replaces the token of the user's Cloud device with the one a boot
    /// minted: the token of an earlier boot opens no connection any more.
    pub(crate) async fn rotate_device_token(&self, device: DeviceId, token: TokenHash) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            let changed = connection.execute(
                "UPDATE devices SET token_hash = ?1 WHERE id = ?2 AND kind = 'managed'",
                [token.as_str(), device.as_str()],
            )?;
            if changed == 0 {
                return Err(StorageError::Corrupt {
                    table: "devices",
                    column: "id",
                    reason: format!("device {device} is not a Cloud device"),
                });
            }
            Ok(())
        })
        .await
    }

    /// Tells every conversation of the user that its Cloud was reset by
    /// `operation`: each one's execution context advances once per reset,
    /// so every node reads the reset in its next context block.
    pub(crate) async fn announce_cloud_reset(&self, user: UserId, operation: OperationId) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "UPDATE conversations SET context_version = context_version + 1, cloud_reset_id = ?2
                 WHERE user_id = ?1 AND (cloud_reset_id IS NULL OR cloud_reset_id <> ?2)",
                [user.as_str(), operation.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// The Cloud reset the conversation was last told of.
    pub(crate) async fn announced_cloud_reset(&self, id: ConversationId) -> Result<Option<OperationId>, StorageError> {
        self.call(move |connection, _| {
            let reset: Option<Option<String>> = connection
                .query_row("SELECT cloud_reset_id FROM conversations WHERE id = ?1", [id.as_str()], |row| {
                    row.get(0)
                })
                .optional()?;
            reset
                .flatten()
                .map(|reset| decode("conversations", "cloud_reset_id", OperationId::try_from(reset)))
                .transpose()
        })
        .await
    }

    /// The user's conversations that are not archived, as the Cloud's
    /// lifecycle weighs them; `cloud` is the user's Cloud device, once its
    /// first use made it.
    pub(crate) async fn cloud_uses(&self, user: UserId, cloud: Option<DeviceId>) -> Result<Vec<CloudUseRecord>, StorageError> {
        const TABLE: &str = "conversations";
        self.call(move |connection, _| {
            let cloud = cloud.as_ref().map(DeviceId::as_str);
            let mut statement = connection.prepare_cached(
                "SELECT c.id, c.target_kind, c.target_device_id, w.device_id AS workspace_device, c.provider_id,
                        EXISTS (SELECT 1 FROM conversation_hosts h
                                WHERE h.conversation_id = c.id AND h.device_id = ?2) AS attached
                 FROM conversations c LEFT JOIN workspaces w ON w.id = c.target_workspace_id
                 WHERE c.user_id = ?1 AND c.archived = 0
                 ORDER BY c.id",
            )?;
            let mut rows = statement.query(params![user.as_str(), cloud])?;
            let mut uses = Vec::new();
            while let Some(row) = rows.next()? {
                let kind: String = row.get("target_kind")?;
                let device: Option<String> = row.get("target_device_id")?;
                let workspace_device: Option<String> = row.get("workspace_device")?;
                let target_device = device.or(workspace_device);
                let on_cloud = kind == "cloud" || (cloud.is_some() && target_device.as_deref() == cloud);
                let provider = row
                    .get::<_, Option<String>>("provider_id")?
                    .map(|provider| decode(TABLE, "provider_id", ProviderId::try_from(provider)))
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

/// An operation's row, read from its columns in `OPERATION_COLUMNS`.
fn operation_row(row: &Row<'_>) -> Result<ManagedOperation, StorageError> {
    Ok(ManagedOperation {
        id: decode(TABLE, "operation_id", OperationId::try_from(row.get::<_, String>("operation_id")?))?,
        base_version: decode(TABLE, "base_version", BaseVersion::parse(row.get::<_, String>("base_version")?))?,
        phase: decode(TABLE, "phase", row.get::<_, String>("phase")?.parse::<ResetPhase>())?,
        error: row.get("error")?,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::storage::control::testing;
    use crate::storage::conversation_index::Creation;

    const FIRST: &str = "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01";
    const SECOND: &str = "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a02";
    const RESET: &str = "5a4b3c2d-1e0f-4a1b-8c2d-3e4f5a6b7c8d";

    #[tokio::test]
    async fn a_resets_intent_is_resumed_and_its_announcement_advances_each_conversation_once() {
        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(&data.path().join("control.sqlite"), Arc::new(demi_core::SystemClock))
            .await
            .unwrap();
        let owner = testing::master(&control).await.id;
        let cloud = control.managed_device_or_create(owner.clone()).await.unwrap().id;
        let reset = OperationId::try_from(RESET).unwrap();
        let operation = |phase, error: Option<&str>| ManagedOperation {
            id: reset.clone(),
            base_version: BaseVersion::parse("base-1").unwrap(),
            phase,
            error: error.map(str::to_owned),
        };
        control
            .put_managed_operation(cloud.clone(), operation(ResetPhase::Rebuilding, None))
            .await
            .unwrap();
        assert_eq!(
            control.unfinished_managed_operations().await.unwrap(),
            [(cloud.clone(), operation(ResetPhase::Rebuilding, None))]
        );
        let failed = operation(ResetPhase::Failed, Some("image publication unavailable"));
        control.put_managed_operation(cloud.clone(), failed.clone()).await.unwrap();
        assert!(control.unfinished_managed_operations().await.unwrap().is_empty());
        assert_eq!(control.managed_operation(cloud.clone(), reset.clone()).await.unwrap(), Some(failed.clone()));
        assert_eq!(control.latest_managed_operation(cloud.clone()).await.unwrap(), Some(failed));

        for id in [FIRST, SECOND] {
            let id = ConversationId::try_from(id).unwrap();
            assert!(matches!(
                control.create_conversation(owner.clone(), id).await.unwrap(),
                Creation::Created(_)
            ));
        }
        // An announcement repeated for the same reset changes nothing more.
        for _ in 0..2 {
            control.announce_cloud_reset(owner.clone(), reset.clone()).await.unwrap();
        }
        for id in [FIRST, SECOND] {
            let id = ConversationId::try_from(id).unwrap();
            let record = control.conversation(id.clone()).await.unwrap().unwrap();
            assert_eq!(record.context_version, 1);
            assert_eq!(control.announced_cloud_reset(id).await.unwrap(), Some(reset.clone()));
        }
        let uses = control.cloud_uses(owner, Some(cloud)).await.unwrap();
        assert!(uses.iter().all(|used| used.on_cloud && !used.attached && used.provider.is_none()));
        assert_eq!(uses.len(), 2);
        control.close().await.unwrap();
    }
}
