//! The reset operations of each user's Cloud
//! (`crates/backend-cloud/src/reset.rs`).

use demi_machine_manager_protocol::BaseVersion;
use demi_web_api_protocol::cloud::ResetPhase;
use demi_web_api_protocol::ids::{DeviceId, OperationId};
use rusqlite::{OptionalExtension, Row, params};

use super::StorageError;
use super::columns::decode;
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
