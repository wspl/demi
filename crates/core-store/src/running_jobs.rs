//! The control database's `running_jobs` index of the jobs of commands that
//! run, by device, written before each job starts and deleted with its
//! command's end. Each command's own record is in its conversation's database
//! (`crates/core-store/src/running.rs`).

use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use rusqlite::params;

use super::StorageError;
use super::columns::decode;
use super::store::CoreStore;

/// A job the control database records running on a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedJob {
    pub job: String,
    /// The conversation whose command it is.
    pub conversation: ConversationId,
    /// Whether its start was sent; one not sent waits for its device.
    pub sent: bool,
}

impl CoreStore {
    /// Records that the job `job` of a command of `conversation` runs on
    /// `device`, before it starts, or, while it is not `sent`, that it waits
    /// to start there (`crates/backend-host-access/src/access.rs`).
    pub async fn record_running_job(
        &self,
        job: String,
        device: DeviceId,
        conversation: ConversationId,
        sent: bool,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "INSERT INTO running_jobs (job_id, device_id, conversation_id, sent) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (job_id) DO UPDATE SET sent = excluded.sent",
                params![job, device.as_str(), conversation.as_str(), sent],
            )?;
            Ok(())
        })
        .await
    }

    /// Forgets the job `job`, whose command ended.
    pub async fn end_running_job(&self, job: String) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute("DELETE FROM running_jobs WHERE job_id = ?1", [job])?;
            Ok(())
        })
        .await
    }

    /// The Clouds whose jobs are recorded running.
    pub async fn clouds_with_running_jobs(&self) -> Result<Vec<DeviceId>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT DISTINCT devices.id FROM running_jobs
                 JOIN devices ON devices.id = running_jobs.device_id
                 WHERE devices.kind = 'managed'",
            )?;
            let mut rows = statement.query([])?;
            let mut clouds = Vec::new();
            while let Some(row) = rows.next()? {
                clouds.push(decode("devices", "id", DeviceId::try_from(row.get::<_, String>(0)?))?);
            }
            Ok(clouds)
        })
        .await
    }

    /// The jobs recorded running on `device`, each with its conversation
    /// and whether it was sent; one that was not waits to start there.
    pub async fn running_jobs(
        &self,
        device: DeviceId,
    ) -> Result<Vec<RecordedJob>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT job_id, conversation_id, sent FROM running_jobs WHERE device_id = ?1",
            )?;
            let mut rows = statement.query([device.as_str()])?;
            let mut jobs = Vec::new();
            while let Some(row) = rows.next()? {
                let conversation = decode(
                    "running_jobs",
                    "conversation_id",
                    ConversationId::try_from(row.get::<_, String>(1)?),
                )?;
                jobs.push(RecordedJob {
                    job: row.get(0)?,
                    conversation,
                    sent: row.get(2)?,
                });
            }
            Ok(jobs)
        })
        .await
    }
}
