//! The records of the commands an integration runs on its Clouds outside any
//! conversation (`crates/backend-user-shard/src/integration_commands.rs`).
//!
//! A row is written when its command starts, again with its job's id before
//! the job is sent to the Cloud, and once more when it ends, with how it
//! ended and its whole output, a blob of the Cloud's user, as an agent's
//! command's row keeps its own (`command_outputs`). A backend that starts
//! again takes up the jobs of the rows that did not end as the Cloud's runner
//! connects; a row with no job belongs to a command whose start an earlier
//! run of the backend never sent. Rows go with their Cloud.

use demi_host_interface::Missing;
use demi_shared_types::{BlobRef, Timestamp};
use demi_web_api_protocol::ids::DeviceId;
use rusqlite::{OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{decode, instant};
use super::control::ControlService;

const COLUMNS: &str =
    "id, device_id, started_at, job_id, ended_at, exit_code, failure, blob, missing_bytes, missing_reason";

/// A command's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudCommandRow {
    pub id: String,
    pub cloud: DeviceId,
    pub started_at: Timestamp,
    /// Its job on the Cloud; none before its start was sent.
    pub job: Option<String>,
    /// How it ended; none while it runs.
    pub end: Option<CloudCommandEnd>,
}

/// How a command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudCommandEnd {
    pub ended_at: Timestamp,
    /// Its exit code; none for one that did not run to its end.
    pub exit_code: Option<i32>,
    /// Why it did not run to its end.
    pub failure: Option<String>,
    /// Its whole output's blob, with the bytes at its end the backend does
    /// not have; none for a command that wrote nothing the backend read.
    pub output: Option<(BlobRef, Option<Missing>)>,
}

impl ControlService {
    /// Records the start of the command `id` on the Cloud `cloud`.
    pub async fn start_cloud_command(&self, id: String, cloud: DeviceId) -> Result<(), StorageError> {
        self.call(move |connection, now| {
            connection.execute(
                "INSERT INTO cloud_commands (id, device_id, started_at) VALUES (?1, ?2, ?3)",
                params![id, cloud.as_str(), now.as_millisecond()],
            )?;
            Ok(())
        })
        .await
    }

    /// Records that the command `id` runs as the job `job`, before the job
    /// is sent.
    pub async fn set_cloud_command_job(&self, id: String, job: String) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute("UPDATE cloud_commands SET job_id = ?2 WHERE id = ?1", params![id, job])?;
            Ok(())
        })
        .await
    }

    /// The rows of the Cloud `cloud` that did not end, oldest first.
    pub async fn running_cloud_commands(&self, cloud: DeviceId) -> Result<Vec<CloudCommandRow>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {COLUMNS} FROM cloud_commands WHERE device_id = ?1 AND ended_at IS NULL ORDER BY started_at"
            ))?;
            let rows = statement
                .query_map([cloud.as_str()], |row| Ok(cloud_command_row(row)))?
                .collect::<Result<Vec<_>, _>>()?;
            rows.into_iter().collect()
        })
        .await
    }

    /// Records how the command `id` ended, unless its row has an end
    /// already: the first end stays.
    pub async fn end_cloud_command(&self, id: String, end: CloudCommandEnd) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            let (blob, missing) = end.output.unzip();
            let missing = missing.flatten();
            connection.execute(
                "UPDATE cloud_commands SET ended_at = ?2, exit_code = ?3, failure = ?4, blob = ?5,
                   missing_bytes = ?6, missing_reason = ?7
                 WHERE id = ?1 AND ended_at IS NULL",
                params![
                    id,
                    end.ended_at.as_millisecond(),
                    end.exit_code,
                    end.failure,
                    blob.as_ref().map(BlobRef::as_str),
                    missing.as_ref().map(|missing| i64::try_from(missing.bytes).unwrap_or(i64::MAX)),
                    missing.map(|missing| missing.reason),
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// The command `id`'s row; none for one that never was, or whose Cloud
    /// is gone.
    pub async fn cloud_command(&self, id: String) -> Result<Option<CloudCommandRow>, StorageError> {
        self.call(move |connection, _| {
            connection
                .query_row(&format!("SELECT {COLUMNS} FROM cloud_commands WHERE id = ?1"), [id], |row| {
                    Ok(cloud_command_row(row))
                })
                .optional()?
                .transpose()
        })
        .await
    }
}

fn cloud_command_row(row: &Row<'_>) -> Result<CloudCommandRow, StorageError> {
    let end = match row.get::<_, Option<i64>>("ended_at")? {
        None => None,
        Some(_) => {
            let blob = row
                .get::<_, Option<String>>("blob")?
                .map(|blob| decode("cloud_commands", "blob", BlobRef::try_from(blob)))
                .transpose()?;
            let missing = match (row.get::<_, Option<i64>>("missing_bytes")?, row.get::<_, Option<String>>("missing_reason")?) {
                (Some(bytes), Some(reason)) => Some(Missing {
                    bytes: decode("cloud_commands", "missing_bytes", u64::try_from(bytes))?,
                    reason,
                }),
                _ => None,
            };
            Some(CloudCommandEnd {
                ended_at: instant(row, "cloud_commands", "ended_at")?,
                exit_code: row.get("exit_code")?,
                failure: row.get("failure")?,
                output: blob.map(|blob| (blob, missing)),
            })
        }
    };
    Ok(CloudCommandRow {
        id: row.get("id")?,
        cloud: decode("cloud_commands", "device_id", DeviceId::try_from(row.get::<_, String>("device_id")?))?,
        started_at: instant(row, "cloud_commands", "started_at")?,
        job: row.get("job_id")?,
        end,
    })
}
