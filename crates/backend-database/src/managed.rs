//! The records of each user's Cloud (`crates/backend-cloud/src/reset.rs`).
//!
//! - Its reset operations.
//! - The token each boot mints.
//! - The announcement of a reset to the user's conversations.
//! - Which of the user's conversations use the Cloud.

use demi_machine_manager_protocol::BaseVersion;
use demi_shared_types::ModelSelection;
use demi_web_api_protocol::cloud::ResetPhase;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, OperationId, ProviderId, UserId};
use rusqlite::{OptionalExtension, Row, params};

use super::StorageError;
use super::accounts::TokenHash;
use super::columns::{decode, json};
use super::control::ControlService;

/// A reset of a Cloud, as its intent is kept: the base it selected when it
/// was admitted, the phase it reached, and why it failed. A retry resumes
/// the same operation on the same base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedOperation {
    pub id: OperationId,
    pub base_version: BaseVersion,
    pub phase: ResetPhase,
    pub error: Option<String>,
}

/// One of the user's conversations as the Cloud's lifecycle weighs it
/// (`crates/backend-cloud/src/uses.rs`): whether
/// its files and commands are on the Cloud, the provider entry it infers
/// with, and whether the Cloud is attached to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudUseRecord {
    pub id: ConversationId,
    pub on_cloud: bool,
    pub provider: Option<ProviderId>,
    pub attached: bool,
}

const TABLE: &str = "managed_operations";
const OPERATION_COLUMNS: &str = "operation_id, base_version, phase, error";

impl ControlService {
    /// The device's reset `id`, as it was last written.
    pub async fn managed_operation(
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
    pub async fn latest_managed_operation(
        &self,
        device: DeviceId,
    ) -> Result<Option<ManagedOperation>, StorageError> {
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
    pub async fn put_managed_operation(
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
    pub async fn unfinished_managed_operations(
        &self,
    ) -> Result<Vec<(DeviceId, ManagedOperation)>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare(&format!(
                "SELECT device_id, {OPERATION_COLUMNS} FROM {TABLE}
                 WHERE phase NOT IN ('ready', 'failed') ORDER BY updated_at, rowid"
            ))?;
            let mut rows = statement.query([])?;
            let mut unfinished = Vec::new();
            while let Some(row) = rows.next()? {
                let device = decode(
                    TABLE,
                    "device_id",
                    DeviceId::try_from(row.get::<_, String>("device_id")?),
                )?;
                unfinished.push((device, operation_row(row)?));
            }
            Ok(unfinished)
        })
        .await
    }

    /// Replaces the token of the user's Cloud device with the one a boot
    /// minted: the token of an earlier boot opens no connection any more.
    pub async fn rotate_device_token(
        &self,
        device: DeviceId,
        token: TokenHash,
    ) -> Result<(), StorageError> {
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

    /// Tells the conversations of the user that reach the Cloud `cloud` that
    /// it was reset by `operation`: each one's execution context advances
    /// once per reset, so every node reads the reset in its next context
    /// block.
    pub async fn announce_cloud_reset(
        &self,
        user: UserId,
        operation: OperationId,
        cloud: DeviceId,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            // Every conversation of the user hears of a reset of their own
            // Cloud, which any of them may reach; one of another Cloud reaches
            // the conversations that work on it or attach it.
            connection.execute(
                "UPDATE conversations SET context_version = context_version + 1, cloud_reset_id = ?2
                 WHERE user_id = ?1 AND (cloud_reset_id IS NULL OR cloud_reset_id <> ?2)
                   AND (?3 = (SELECT cloud_device_id FROM users WHERE id = ?1)
                     OR target_device_id = ?3
                     OR target_project_id IN (SELECT id FROM projects WHERE device_id = ?3)
                     OR id IN (SELECT conversation_id FROM conversation_hosts WHERE device_id = ?3))",
                [user.as_str(), operation.as_str(), cloud.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// The Cloud reset the conversation was last told of.
    pub async fn announced_cloud_reset(
        &self,
        id: ConversationId,
    ) -> Result<Option<OperationId>, StorageError> {
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
                .map(|reset| {
                    decode(
                        "conversations",
                        "cloud_reset_id",
                        OperationId::try_from(reset),
                    )
                })
                .transpose()
        })
        .await
    }

    /// The user's conversations that are not archived, as the lifecycle of
    /// the Cloud `cloud` weighs them: the user's own Cloud when `own`, which a
    /// conversation targets by kind, once its first use made it; another
    /// Cloud of the user's, which a conversation targets by its device.
    pub async fn cloud_uses(
        &self,
        user: UserId,
        cloud: Option<DeviceId>,
        own: bool,
    ) -> Result<Vec<CloudUseRecord>, StorageError> {
        const TABLE: &str = "conversations";
        self.call(move |connection, _| {
            let cloud = cloud.as_ref().map(DeviceId::as_str);
            let mut statement = connection.prepare_cached(
                "SELECT c.id, c.target_kind, c.target_device_id, w.device_id AS project_device, c.model,
                        EXISTS (SELECT 1 FROM conversation_hosts h
                                WHERE h.conversation_id = c.id AND h.device_id = ?2) AS attached
                 FROM conversations c LEFT JOIN projects w ON w.id = c.target_project_id
                 WHERE c.user_id = ?1 AND c.archived = 0
                 ORDER BY c.id",
            )?;
            let mut rows = statement.query(params![user.as_str(), cloud])?;
            let mut uses = Vec::new();
            while let Some(row) = rows.next()? {
                let kind: String = row.get("target_kind")?;
                let device: Option<String> = row.get("target_device_id")?;
                let project_device: Option<String> = row.get("project_device")?;
                let target_device = device.or(project_device);
                let on_cloud = (own && kind == "cloud") || (cloud.is_some() && target_device.as_deref() == cloud);
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

/// An operation's row, read from its columns in `OPERATION_COLUMNS`.
fn operation_row(row: &Row<'_>) -> Result<ManagedOperation, StorageError> {
    Ok(ManagedOperation {
        id: decode(
            TABLE,
            "operation_id",
            OperationId::try_from(row.get::<_, String>("operation_id")?),
        )?,
        base_version: decode(
            TABLE,
            "base_version",
            BaseVersion::parse(row.get::<_, String>("base_version")?),
        )?,
        phase: decode(
            TABLE,
            "phase",
            row.get::<_, String>("phase")?.parse::<ResetPhase>(),
        )?,
        error: row.get("error")?,
    })
}
