//! The records of commands that run (`storage.md` § Command outputs): the
//! conversation's `running_commands` row of each, written when its job
//! starts and deleted in the transaction that writes its `command_outputs`
//! row, and the control database's `running_jobs` index of their jobs by
//! device, written before each job starts and deleted with its command's
//! end. A backend that starts again reads them to take the commands up when
//! their runners connect (`sessions-and-targets.md` § Recovery and
//! persistence).

use demi_shared_types::{CommandId, NodeId, Timestamp};
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{decode, instant};
use super::command_outputs::{self, CommandOutput};
use super::control::ControlService;

/// A command that runs, as its conversation records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningCommand {
    pub command: CommandId,
    /// The node that ran it.
    pub node: NodeId,
    pub device: DeviceId,
    /// Its job's id on that device.
    pub job: String,
    /// The `shell` call that started it.
    pub tool_use_id: String,
    pub started: Timestamp,
}

const COLUMNS: &str = "command_id, node_id, device_id, job_id, tool_use_id, started_at";

/// Records `running`; a command recorded already keeps its row.
pub fn insert(connection: &Connection, running: &RunningCommand) -> Result<(), StorageError> {
    connection
        .prepare_cached(&format!(
            "INSERT INTO running_commands ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (command_id) DO NOTHING"
        ))?
        .execute(params![
            running.command.as_str(),
            running.node.as_str(),
            running.device.as_str(),
            running.job,
            running.tool_use_id,
            running.started.as_millisecond(),
        ])?;
    Ok(())
}

/// Records the command's end, `output`, in place of its record as running,
/// in one transaction; answers the job it ran as, when it was recorded
/// running.
pub fn end(connection: &mut Connection, output: CommandOutput) -> Result<Option<String>, StorageError> {
    let transaction = connection.transaction()?;
    let job = transaction
        .prepare_cached("DELETE FROM running_commands WHERE command_id = ?1 RETURNING job_id")?
        .query_row([output.command.as_str()], |row| row.get(0))
        .optional()?;
    command_outputs::insert_in(&transaction, &[output])?;
    transaction.commit()?;
    Ok(job)
}

/// The commands `node` ran that run, oldest first.
pub fn of_node(connection: &Connection, node: &NodeId) -> Result<Vec<RunningCommand>, StorageError> {
    let mut statement = connection.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM running_commands WHERE node_id = ?1 ORDER BY started_at, command_id"
    ))?;
    let mut rows = statement.query([node.as_str()])?;
    let mut running = Vec::new();
    while let Some(row) = rows.next()? {
        running.push(decode_row(row)?);
    }
    Ok(running)
}

fn decode_row(row: &Row<'_>) -> Result<RunningCommand, StorageError> {
    let table = "running_commands";
    Ok(RunningCommand {
        command: decode(table, "command_id", CommandId::try_from(row.get::<_, String>(0)?))?,
        node: decode(table, "node_id", NodeId::try_from(row.get::<_, String>(1)?))?,
        device: decode(table, "device_id", DeviceId::try_from(row.get::<_, String>(2)?))?,
        job: row.get(3)?,
        tool_use_id: row.get(4)?,
        started: instant(row, table, "started_at")?,
    })
}

impl ControlService {
    /// Records that the job `job` of a command of `conversation` runs on
    /// `device`, before it starts.
    pub async fn record_running_job(
        &self,
        job: String,
        device: DeviceId,
        conversation: ConversationId,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "INSERT INTO running_jobs (job_id, device_id, conversation_id) VALUES (?1, ?2, ?3)
                 ON CONFLICT (job_id) DO NOTHING",
                params![job, device.as_str(), conversation.as_str()],
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

    /// The jobs recorded running on `device`, each with its conversation.
    pub async fn running_jobs(
        &self,
        device: DeviceId,
    ) -> Result<Vec<(String, ConversationId)>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT job_id, conversation_id FROM running_jobs WHERE device_id = ?1",
            )?;
            let mut rows = statement.query([device.as_str()])?;
            let mut jobs = Vec::new();
            while let Some(row) = rows.next()? {
                let conversation = decode(
                    "running_jobs",
                    "conversation_id",
                    ConversationId::try_from(row.get::<_, String>(1)?),
                )?;
                jobs.push((row.get(0)?, conversation));
            }
            Ok(jobs)
        })
        .await
    }
}
