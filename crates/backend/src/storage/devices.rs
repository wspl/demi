//! Device records (`storage.md` § Control records): who owns a device, how
//! it came to be, and the hash of its current token, which finds the device
//! of a runner's hello or pipe request through one index lookup. Whether a
//! device is online is its runner connection's, not a record's.

use demi_core::Timestamp;
use demi_runner_protocol::wire::RunnerPlatform;
use demi_web_api::devices::DeviceKind;
use demi_web_api::ids::{DeviceId, UserId};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{decode, instant};
use super::control::ControlService;
use crate::auth::sessions::TokenHash;

/// A `devices` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeviceRecord {
    pub(crate) id: DeviceId,
    pub(crate) user: UserId,
    pub(crate) kind: DeviceKind,
    pub(crate) name: String,
    pub(crate) platform: RunnerPlatform,
    pub(crate) claimed_at: Timestamp,
    pub(crate) last_seen_at: Option<Timestamp>,
}

const DEVICE_COLUMNS: &str = "id, user_id, kind, name, platform, claimed_at, last_seen_at";

/// The name and platform of the one device a user's Cloud is.
const CLOUD_NAME: &str = "Cloud";
const CLOUD_PLATFORM: RunnerPlatform = RunnerPlatform::Linux;

impl ControlService {
    /// Stores a device the user paired, with the hash of the token its
    /// runner receives.
    pub(crate) async fn create_device(
        &self,
        user: UserId,
        name: String,
        platform: RunnerPlatform,
        token: TokenHash,
    ) -> Result<DeviceRecord, StorageError> {
        let id = DeviceId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        self.call(move |connection, now| {
            connection.execute(
                "INSERT INTO devices (id, user_id, kind, name, platform, token_hash, claimed_at, last_seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
                params![
                    id.as_str(),
                    user.as_str(),
                    DeviceKind::User.to_string(),
                    name,
                    platform.to_string(),
                    token.as_str(),
                    now.as_millisecond()
                ],
            )?;
            Ok(DeviceRecord {
                id,
                user,
                kind: DeviceKind::User,
                name,
                platform,
                claimed_at: now,
                last_seen_at: None,
            })
        })
        .await
    }

    pub(crate) async fn device(&self, id: DeviceId) -> Result<Option<DeviceRecord>, StorageError> {
        self.call(move |connection, _| one(connection, "id = ?1", id.as_str()))
            .await
    }

    /// The device whose current token has this hash.
    pub(crate) async fn device_by_token(&self, token: TokenHash) -> Result<Option<DeviceRecord>, StorageError> {
        self.call(move |connection, _| one(connection, "token_hash = ?1", token.as_str()))
            .await
    }

    /// The user's Cloud device, when its first use made it.
    pub(crate) async fn managed_device(&self, user: UserId) -> Result<Option<DeviceRecord>, StorageError> {
        self.call(move |connection, _| one(connection, "kind = 'managed' AND user_id = ?1", user.as_str()))
            .await
    }

    /// The user's Cloud device, made on its first use. The partial unique
    /// index admits one per user, so concurrent first uses find the same
    /// one. Its token is issued when it boots.
    pub(crate) async fn managed_device_or_create(&self, user: UserId) -> Result<DeviceRecord, StorageError> {
        let id = DeviceId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "INSERT INTO devices (id, user_id, kind, name, platform, token_hash, claimed_at, last_seen_at)
                 VALUES (?1, ?2, 'managed', ?3, ?4, NULL, ?5, NULL)
                 ON CONFLICT DO NOTHING",
                params![id.as_str(), user.as_str(), CLOUD_NAME, CLOUD_PLATFORM.to_string(), now.as_millisecond()],
            )?;
            let device = one(&transaction, "kind = 'managed' AND user_id = ?1", user.as_str())?;
            transaction.commit()?;
            device.ok_or(StorageError::Corrupt {
                table: "devices",
                column: "kind",
                reason: "the user's Cloud device was neither found nor made".into(),
            })
        })
        .await
    }

    /// The devices the user paired, oldest first.
    pub(crate) async fn paired_devices(&self, user: UserId) -> Result<Vec<DeviceRecord>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {DEVICE_COLUMNS} FROM devices WHERE user_id = ?1 AND kind = 'user' ORDER BY claimed_at, id"
            ))?;
            let mut rows = statement.query([user.as_str()])?;
            let mut devices = Vec::new();
            while let Some(row) = rows.next()? {
                devices.push(device_row(row)?);
            }
            Ok(devices)
        })
        .await
    }

    /// How many workspaces point at the device.
    pub(crate) async fn workspaces_on_device(&self, device: DeviceId) -> Result<u64, StorageError> {
        self.call(move |connection, _| {
            let count: i64 = connection.query_row(
                "SELECT COUNT(*) FROM workspaces WHERE device_id = ?1",
                [device.as_str()],
                |row| row.get(0),
            )?;
            decode("workspaces", "device_id", u64::try_from(count))
        })
        .await
    }

    /// Deletes the device with its attachments to conversations; its
    /// exposes go with it.
    pub(crate) async fn delete_device(&self, device: DeviceId) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            transaction.execute("DELETE FROM conversation_hosts WHERE device_id = ?1", [device.as_str()])?;
            transaction.execute("DELETE FROM devices WHERE id = ?1", [device.as_str()])?;
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    /// Records that the device's runner was connected just now.
    pub(crate) async fn touch_device_seen(&self, device: DeviceId) -> Result<(), StorageError> {
        self.call(move |connection, now| {
            connection.execute(
                "UPDATE devices SET last_seen_at = ?1 WHERE id = ?2",
                params![now.as_millisecond(), device.as_str()],
            )?;
            Ok(())
        })
        .await
    }
}

/// The one device `condition` selects with `value`.
fn one(connection: &Connection, condition: &str, value: &str) -> Result<Option<DeviceRecord>, StorageError> {
    let mut statement = connection.prepare_cached(&format!("SELECT {DEVICE_COLUMNS} FROM devices WHERE {condition}"))?;
    statement
        .query_row([value], |row| Ok(device_row(row)))
        .optional()?
        .transpose()
}

/// A `devices` row, read from its columns in `DEVICE_COLUMNS`.
fn device_row(row: &Row<'_>) -> Result<DeviceRecord, StorageError> {
    let last_seen_at = match row.get::<_, Option<i64>>("last_seen_at")? {
        Some(millisecond) => Some(decode("devices", "last_seen_at", Timestamp::from_millisecond(millisecond))?),
        None => None,
    };
    Ok(DeviceRecord {
        id: decode("devices", "id", DeviceId::try_from(row.get::<_, String>("id")?))?,
        user: decode("devices", "user_id", UserId::try_from(row.get::<_, String>("user_id")?))?,
        kind: decode("devices", "kind", row.get::<_, String>("kind")?.parse::<DeviceKind>())?,
        name: row.get("name")?,
        platform: decode("devices", "platform", row.get::<_, String>("platform")?.parse::<RunnerPlatform>())?,
        claimed_at: instant(row, "devices", "claimed_at")?,
        last_seen_at,
    })
}
