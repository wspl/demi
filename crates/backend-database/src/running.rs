//! The records of commands that run (`storage.md` § Command outputs): the
//! conversation's `running_commands` row of each, written when its job
//! starts and deleted in the transaction that writes its `command_outputs`
//! row, and the control database's `running_jobs` index of their jobs by
//! device, written before each job starts and deleted with its command's
//! end. A backend that starts again reads them to take the commands up when
//! their runners connect (`sessions-and-targets.md` § Recovery and
//! persistence).

use std::collections::BTreeMap;

use demi_host_interface::Seen;
use demi_shared_types::{CommandId, NodeId, Timestamp};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::StorageError;
use super::columns::{decode, instant, json, to_json};
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
    /// How far each node that looked at it has seen its output, which a
    /// look or a report moves.
    pub places: BTreeMap<NodeId, Seen>,
}

const COLUMNS: &str = "command_id, node_id, device_id, job_id, tool_use_id, started_at, places";

/// How far one node has seen a running command's output, as the `places`
/// column stores it (command places format 1): any count of bytes is one.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPlace {
    stdout: u64,
    stderr: u64,
}

/// The `places` column: each node's place, by the node.
#[derive(Serialize, Deserialize, garde::Validate)]
#[serde(transparent)]
struct StoredPlaces(#[garde(skip)] BTreeMap<NodeId, StoredPlace>);

impl StoredPlaces {
    fn of(places: &BTreeMap<NodeId, Seen>) -> Self {
        Self(
            places
                .iter()
                .map(|(node, seen)| {
                    let place = StoredPlace {
                        stdout: seen.stdout,
                        stderr: seen.stderr,
                    };
                    (node.clone(), place)
                })
                .collect(),
        )
    }

    fn places(self) -> BTreeMap<NodeId, Seen> {
        self.0
            .into_iter()
            .map(|(node, place)| {
                let seen = Seen {
                    stdout: place.stdout,
                    stderr: place.stderr,
                };
                (node, seen)
            })
            .collect()
    }
}

/// Records `running`; a command recorded already keeps its row.
pub fn insert(connection: &Connection, running: &RunningCommand) -> Result<(), StorageError> {
    connection
        .prepare_cached(&format!(
            "INSERT INTO running_commands ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (command_id) DO NOTHING"
        ))?
        .execute(params![
            running.command.as_str(),
            running.node.as_str(),
            running.device.as_str(),
            running.job,
            running.tool_use_id,
            running.started.as_millisecond(),
            to_json(&StoredPlaces::of(&running.places)),
        ])?;
    Ok(())
}

/// Records that `node` has seen `command`'s output as far as `seen`, in
/// one transaction; nothing for a command that does not run.
pub fn set_place(
    connection: &mut Connection,
    command: &CommandId,
    node: &NodeId,
    seen: Seen,
) -> Result<(), StorageError> {
    let transaction = connection.transaction()?;
    let stored: Option<String> = transaction
        .prepare_cached("SELECT places FROM running_commands WHERE command_id = ?1")?
        .query_row([command.as_str()], |row| row.get(0))
        .optional()?;
    let Some(stored) = stored else {
        return Ok(());
    };
    let mut places = json::<StoredPlaces>("running_commands", "places", &stored)?.places();
    places.insert(node.clone(), seen);
    transaction
        .prepare_cached("UPDATE running_commands SET places = ?2 WHERE command_id = ?1")?
        .execute(params![command.as_str(), to_json(&StoredPlaces::of(&places))])?;
    transaction.commit()?;
    Ok(())
}

/// How far `node` has seen `command`'s output, as its record keeps it;
/// none when the node has not looked, or the command does not run.
pub fn place(connection: &Connection, command: &CommandId, node: &NodeId) -> Result<Option<Seen>, StorageError> {
    let stored: Option<String> = connection
        .prepare_cached("SELECT places FROM running_commands WHERE command_id = ?1")?
        .query_row([command.as_str()], |row| row.get(0))
        .optional()?;
    let Some(stored) = stored else {
        return Ok(None);
    };
    Ok(json::<StoredPlaces>("running_commands", "places", &stored)?
        .places()
        .remove(node))
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
        places: json::<StoredPlaces>(table, "places", &row.get::<_, String>(6)?)?.places(),
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

    /// The Clouds whose jobs are recorded running, each with its owner.
    pub async fn clouds_with_running_jobs(&self) -> Result<Vec<(DeviceId, UserId)>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT DISTINCT devices.id, devices.user_id FROM running_jobs
                 JOIN devices ON devices.id = running_jobs.device_id
                 WHERE devices.kind = 'managed'",
            )?;
            let mut rows = statement.query([])?;
            let mut clouds = Vec::new();
            while let Some(row) = rows.next()? {
                let device = decode("devices", "id", DeviceId::try_from(row.get::<_, String>(0)?))?;
                let user = decode("devices", "user_id", UserId::try_from(row.get::<_, String>(1)?))?;
                clouds.push((device, user));
            }
            Ok(clouds)
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
