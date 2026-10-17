//! The users' port forwards (`storage.md` § Control records,
//! `port-forwarding.md`): each forward's name, unique among its user's, its
//! Listen On device and port, its Connect To device, none until chosen, and
//! address, and its switch. What a forward is doing now is its Listen On
//! runner's, not a record's.

use demi_shared_types::Timestamp;
use demi_web_api_protocol::ids::{DeviceId, PortForwardId, UserId};
use demi_web_api_protocol::port_forwards::{
    DEFAULT_ADDRESS, DEFAULT_NAME, DEFAULT_PORT, ForwardAddress, name_taken,
};
use rusqlite::{OptionalExtension, Row, Transaction, params};

use super::StorageError;
use super::columns::{decode, instant};
use super::control::ControlService;

const TABLE: &str = "port_forwards";

/// A `port_forwards` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortForwardRecord {
    pub id: PortForwardId,
    pub name: String,
    pub listen_device: DeviceId,
    pub listen_port: u16,
    pub connect_device: Option<DeviceId>,
    pub connect_address: ForwardAddress,
    pub enabled: bool,
    pub created_at: Timestamp,
}

/// What a change sets; a new forward takes each over its default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortForwardChange {
    pub name: Option<String>,
    pub listen_device: Option<DeviceId>,
    pub listen_port: Option<u16>,
    /// `Some(None)` leaves the Connect To Host to be chosen.
    pub connect_device: Option<Option<DeviceId>>,
    pub connect_address: Option<ForwardAddress>,
    pub enabled: Option<bool>,
}

/// Why a forward was not stored.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortForwardRefusal {
    #[error("No such port forward")]
    NotFound,
    /// Another of the user's forwards has the name.
    #[error("{}", name_taken(.0))]
    NameTaken(String),
    /// A device that is not one of the user's Hosts.
    #[error("No such Host")]
    NoSuchHost,
    /// A new forward with no Listen On given, for a user who has no Host.
    #[error("Add a device or the Cloud first")]
    NoHost,
}

/// What a change answers: the forward as stored, or why it was not.
pub type Stored = Result<PortForwardRecord, PortForwardRefusal>;

fn record(row: &Row<'_>) -> Result<PortForwardRecord, StorageError> {
    let port: i64 = row.get("listen_port")?;
    Ok(PortForwardRecord {
        id: decode(TABLE, "id", PortForwardId::try_from(row.get::<_, String>("id")?))?,
        name: row.get("name")?,
        listen_device: decode(
            TABLE,
            "listen_device_id",
            DeviceId::try_from(row.get::<_, String>("listen_device_id")?),
        )?,
        listen_port: decode(TABLE, "listen_port", u16::try_from(port))?,
        connect_device: row
            .get::<_, Option<String>>("connect_device_id")?
            .map(|id| decode(TABLE, "connect_device_id", DeviceId::try_from(id)))
            .transpose()?,
        connect_address: decode(
            TABLE,
            "connect_address",
            ForwardAddress::try_from(row.get::<_, String>("connect_address")?),
        )?,
        enabled: row.get("enabled")?,
        created_at: instant(row, TABLE, "created_at")?,
    })
}

const COLUMNS: &str = "id, name, listen_device_id, listen_port, connect_device_id, connect_address, enabled, created_at";

/// The user's forward `id`.
fn by_id(
    transaction: &Transaction<'_>,
    user: &UserId,
    id: &PortForwardId,
) -> Result<Option<PortForwardRecord>, StorageError> {
    transaction
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM port_forwards WHERE id = ?1 AND user_id = ?2"
        ))?
        .query_row(params![id.as_str(), user.as_str()], |row| Ok(record(row)))
        .optional()?
        .transpose()
}

/// Whether `device` is one of `user`'s Hosts: a device they paired or their
/// Cloud, never the Claude Code machine.
pub(crate) fn is_host(transaction: &Transaction<'_>, user: &UserId, device: &DeviceId) -> Result<bool, StorageError> {
    Ok(transaction.query_row(
        "SELECT EXISTS (SELECT 1 FROM devices WHERE id = ?1 AND user_id = ?2
           AND (machine IS NULL OR machine = 'cloud'))",
        params![device.as_str(), user.as_str()],
        |row| row.get(0),
    )?)
}

/// Whether another of `user`'s forwards than `except` is named `name`.
fn name_used(
    transaction: &Transaction<'_>,
    user: &UserId,
    name: &str,
    except: Option<&PortForwardId>,
) -> Result<bool, StorageError> {
    Ok(transaction.query_row(
        "SELECT EXISTS (SELECT 1 FROM port_forwards WHERE user_id = ?1 AND name = ?2 AND id IS NOT ?3)",
        params![user.as_str(), name, except.map(PortForwardId::as_str)],
        |row| row.get(0),
    )?)
}

impl ControlService {
    /// The user's forwards, oldest first.
    pub async fn port_forwards(&self, user: UserId) -> Result<Vec<PortForwardRecord>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {COLUMNS} FROM port_forwards WHERE user_id = ?1 ORDER BY created_at, id"
            ))?;
            let mut rows = statement.query([user.as_str()])?;
            let mut forwards = Vec::new();
            while let Some(row) = rows.next()? {
                forwards.push(record(row)?);
            }
            Ok(forwards)
        })
        .await
    }

    /// Makes a forward for `user` from its defaults, *Forward*, *Forward
    /// (2)* for the second, on, listening on `first_host` on port 8080 or
    /// the next port none of the user's forwards there uses, connecting to
    /// `localhost:11434` on no Host yet, with `change` over them.
    pub async fn create_port_forward(
        &self,
        user: UserId,
        first_host: Option<DeviceId>,
        change: PortForwardChange,
    ) -> Result<Stored, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let Some(listen_device) = change.listen_device.clone().or(first_host) else {
                return Ok(Err(PortForwardRefusal::NoHost));
            };
            let name = match change.name.clone() {
                Some(name) => {
                    if name_used(&transaction, &user, &name, None)? {
                        return Ok(Err(PortForwardRefusal::NameTaken(name)));
                    }
                    name
                }
                None => {
                    let mut number = 1;
                    loop {
                        let name = match number {
                            1 => DEFAULT_NAME.to_owned(),
                            n => format!("{DEFAULT_NAME} ({n})"),
                        };
                        if !name_used(&transaction, &user, &name, None)? {
                            break name;
                        }
                        number += 1;
                    }
                }
            };
            let listen_port = match change.listen_port {
                Some(port) => port,
                None => {
                    let used: Vec<i64> = transaction
                        .prepare_cached(
                            "SELECT listen_port FROM port_forwards WHERE user_id = ?1 AND listen_device_id = ?2",
                        )?
                        .query_map(params![user.as_str(), listen_device.as_str()], |row| row.get(0))?
                        .collect::<Result<_, _>>()?;
                    (DEFAULT_PORT..=u16::MAX)
                        .find(|port| !used.contains(&i64::from(*port)))
                        .unwrap_or(DEFAULT_PORT)
                }
            };
            let connect_device = change.connect_device.clone().flatten();
            for device in std::iter::once(&listen_device).chain(connect_device.as_ref()) {
                if !is_host(&transaction, &user, device)? {
                    return Ok(Err(PortForwardRefusal::NoSuchHost));
                }
            }
            let address = change.connect_address.clone().unwrap_or_else(|| {
                ForwardAddress::try_from(DEFAULT_ADDRESS.to_owned()).expect("the default address is one")
            });
            let id = PortForwardId::try_from(format!("fwd-{}", uuid::Uuid::new_v4().simple()))
                .expect("a generated id is nonempty");
            transaction.execute(
                "INSERT INTO port_forwards (id, user_id, name, listen_device_id, listen_port,
                   connect_device_id, connect_address, enabled, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id.as_str(),
                    user.as_str(),
                    name,
                    listen_device.as_str(),
                    listen_port,
                    connect_device.as_ref().map(DeviceId::as_str),
                    address.as_str(),
                    change.enabled.unwrap_or(true),
                    now.as_millisecond(),
                ],
            )?;
            let stored = by_id(&transaction, &user, &id)?.expect("the forward was just stored");
            transaction.commit()?;
            Ok(Ok(stored))
        })
        .await
    }

    /// Changes the user's forward `id` by `change`.
    pub async fn change_port_forward(
        &self,
        user: UserId,
        id: PortForwardId,
        change: PortForwardChange,
    ) -> Result<Stored, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let Some(mut forward) = by_id(&transaction, &user, &id)? else {
                return Ok(Err(PortForwardRefusal::NotFound));
            };
            if let Some(name) = change.name {
                if name_used(&transaction, &user, &name, Some(&id))? {
                    return Ok(Err(PortForwardRefusal::NameTaken(name)));
                }
                forward.name = name;
            }
            if let Some(device) = change.listen_device {
                forward.listen_device = device;
            }
            if let Some(port) = change.listen_port {
                forward.listen_port = port;
            }
            if let Some(device) = change.connect_device {
                forward.connect_device = device;
            }
            if let Some(address) = change.connect_address {
                forward.connect_address = address;
            }
            if let Some(enabled) = change.enabled {
                forward.enabled = enabled;
            }
            for device in std::iter::once(&forward.listen_device).chain(forward.connect_device.as_ref()) {
                if !is_host(&transaction, &user, device)? {
                    return Ok(Err(PortForwardRefusal::NoSuchHost));
                }
            }
            transaction.execute(
                "UPDATE port_forwards SET name = ?3, listen_device_id = ?4, listen_port = ?5,
                   connect_device_id = ?6, connect_address = ?7, enabled = ?8
                 WHERE id = ?1 AND user_id = ?2",
                params![
                    id.as_str(),
                    user.as_str(),
                    forward.name,
                    forward.listen_device.as_str(),
                    forward.listen_port,
                    forward.connect_device.as_ref().map(DeviceId::as_str),
                    forward.connect_address.as_str(),
                    forward.enabled,
                ],
            )?;
            transaction.commit()?;
            Ok(Ok(forward))
        })
        .await
    }

    /// Removes the user's forward `id`; false when they have none.
    pub async fn remove_port_forward(&self, user: UserId, id: PortForwardId) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let removed = connection.execute(
                "DELETE FROM port_forwards WHERE id = ?1 AND user_id = ?2",
                params![id.as_str(), user.as_str()],
            )?;
            Ok(removed > 0)
        })
        .await
    }
}
