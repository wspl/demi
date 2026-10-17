//! The users' exposed ports (`storage.md` § Control records,
//! `port-exposure.md`): each a port of one of the user's Hosts, missing
//! until the user enters it, and its label. A request the backend makes for
//! the user reaches a device's port only while a row exposes it.

use demi_shared_types::Timestamp;
use demi_web_api_protocol::exposed_ports::port_taken;
use demi_web_api_protocol::ids::{DeviceId, ExposedPortId, UserId};
use rusqlite::{OptionalExtension, Row, Transaction, params};

use super::StorageError;
use super::columns::{decode, instant};
use super::control::ControlService;
use super::port_forwards::is_host;

const TABLE: &str = "exposed_ports";

/// An `exposed_ports` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExposedPortRecord {
    pub id: ExposedPortId,
    pub device: DeviceId,
    /// None until the user enters it: the row exposes nothing meanwhile.
    pub port: Option<u16>,
    pub label: String,
    pub created_at: Timestamp,
}

/// What a change sets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExposedPortChange {
    /// `Some(None)` leaves the port to be entered.
    pub port: Option<Option<u16>>,
    pub label: Option<String>,
}

/// Why an exposed port was not stored.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExposedPortRefusal {
    #[error("No such exposed port")]
    NotFound,
    /// A device that is not one of the user's Hosts.
    #[error("No such Host")]
    NoSuchHost,
    /// Another row of the device exposes the port.
    #[error("{}", port_taken(*.0))]
    PortTaken(u16),
}

/// What a change answers: the row as stored, or why it was not.
pub type Stored = Result<ExposedPortRecord, ExposedPortRefusal>;

/// What exposing a port answers: the row that exposes it, and whether this
/// call made it rather than finding it exposed already.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub exposed: ExposedPortRecord,
    pub new: bool,
}

fn record(row: &Row<'_>) -> Result<ExposedPortRecord, StorageError> {
    Ok(ExposedPortRecord {
        id: decode(TABLE, "id", ExposedPortId::try_from(row.get::<_, String>("id")?))?,
        device: decode(TABLE, "device_id", DeviceId::try_from(row.get::<_, String>("device_id")?))?,
        port: row
            .get::<_, Option<i64>>("port")?
            .map(|port| decode(TABLE, "port", u16::try_from(port)))
            .transpose()?,
        label: row.get("label")?,
        created_at: instant(row, TABLE, "created_at")?,
    })
}

const COLUMNS: &str = "id, device_id, port, label, created_at";

fn by_id(
    transaction: &Transaction<'_>,
    user: &UserId,
    id: &ExposedPortId,
) -> Result<Option<ExposedPortRecord>, StorageError> {
    transaction
        .prepare_cached(&format!("SELECT {COLUMNS} FROM exposed_ports WHERE id = ?1 AND user_id = ?2"))?
        .query_row(params![id.as_str(), user.as_str()], |row| Ok(record(row)))
        .optional()?
        .transpose()
}

/// The user's row of `device` that exposes `port`.
fn with_port(
    transaction: &Transaction<'_>,
    user: &UserId,
    device: &DeviceId,
    port: u16,
) -> Result<Option<ExposedPortRecord>, StorageError> {
    transaction
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM exposed_ports WHERE user_id = ?1 AND device_id = ?2 AND port = ?3"
        ))?
        .query_row(params![user.as_str(), device.as_str(), port], |row| Ok(record(row)))
        .optional()?
        .transpose()
}

/// Whether another row of `device` than `except` exposes `port`.
fn port_used(
    transaction: &Transaction<'_>,
    device: &DeviceId,
    port: u16,
    except: Option<&ExposedPortId>,
) -> Result<bool, StorageError> {
    Ok(transaction.query_row(
        "SELECT EXISTS (SELECT 1 FROM exposed_ports WHERE device_id = ?1 AND port = ?2 AND id IS NOT ?3)",
        params![device.as_str(), port, except.map(ExposedPortId::as_str)],
        |row| row.get(0),
    )?)
}

impl ControlService {
    /// The user's exposed ports, oldest first.
    pub async fn exposed_ports(&self, user: UserId) -> Result<Vec<ExposedPortRecord>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {COLUMNS} FROM exposed_ports WHERE user_id = ?1 ORDER BY created_at, id"
            ))?;
            let mut rows = statement.query([user.as_str()])?;
            let mut ports = Vec::new();
            while let Some(row) = rows.next()? {
                ports.push(record(row)?);
            }
            Ok(ports)
        })
        .await
    }

    /// Exposes a port of `device`, one of the user's Hosts: `port`, or
    /// none yet, with `label`. A port the device exposes already answers
    /// its row as it is (`port-exposure.md` § Exposed ports, Recovery).
    pub async fn create_exposed_port(
        &self,
        user: UserId,
        device: DeviceId,
        port: Option<u16>,
        label: String,
    ) -> Result<Result<Created, ExposedPortRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            if !is_host(&transaction, &user, &device)? {
                return Ok(Err(ExposedPortRefusal::NoSuchHost));
            }
            // Exposing a port that is exposed already is done: the port is
            // what the caller wanted, whoever exposed it meanwhile.
            if let Some(port) = port
                && let Some(exposed) = with_port(&transaction, &user, &device, port)?
            {
                return Ok(Ok(Created { exposed, new: false }));
            }
            let id = ExposedPortId::try_from(format!("exp-{}", uuid::Uuid::new_v4().simple()))
                .expect("a generated id is nonempty");
            transaction.execute(
                "INSERT INTO exposed_ports (id, user_id, device_id, port, label, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id.as_str(), user.as_str(), device.as_str(), port, label, now.as_millisecond()],
            )?;
            let exposed = by_id(&transaction, &user, &id)?.expect("the exposed port was just stored");
            transaction.commit()?;
            Ok(Ok(Created { exposed, new: true }))
        })
        .await
    }

    /// Changes the user's exposed port `id` by `change`.
    pub async fn change_exposed_port(
        &self,
        user: UserId,
        id: ExposedPortId,
        change: ExposedPortChange,
    ) -> Result<Stored, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let Some(mut exposed) = by_id(&transaction, &user, &id)? else {
                return Ok(Err(ExposedPortRefusal::NotFound));
            };
            if let Some(port) = change.port {
                if let Some(port) = port
                    && port_used(&transaction, &exposed.device, port, Some(&id))?
                {
                    return Ok(Err(ExposedPortRefusal::PortTaken(port)));
                }
                exposed.port = port;
            }
            if let Some(label) = change.label {
                exposed.label = label;
            }
            transaction.execute(
                "UPDATE exposed_ports SET port = ?3, label = ?4 WHERE id = ?1 AND user_id = ?2",
                params![id.as_str(), user.as_str(), exposed.port, exposed.label],
            )?;
            transaction.commit()?;
            Ok(Ok(exposed))
        })
        .await
    }

    /// Removes the user's exposed port `id`; false when they have none.
    pub async fn remove_exposed_port(&self, user: UserId, id: ExposedPortId) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let removed = connection.execute(
                "DELETE FROM exposed_ports WHERE id = ?1 AND user_id = ?2",
                params![id.as_str(), user.as_str()],
            )?;
            Ok(removed > 0)
        })
        .await
    }
}
