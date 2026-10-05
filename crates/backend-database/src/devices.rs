//! Device records (`storage.md` § Control records): who owns a device, how
//! it came to be, and the hash of its current token, which finds the device
//! of a runner's hello or pipe request through one index lookup, and what
//! its runner last reported its artifact cache holds. Whether a device is
//! online is its runner connection's, not a record's.

use demi_runner_protocol::wire::{HostArtifact, RunnerPlatform};
use demi_shared_types::Timestamp;
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::accounts::TokenHash;
use super::columns::{decode, instant, json, to_json};
use super::control::ControlService;

/// What a device's deletion removed with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRemoval {
    /// The workspaces whose device it was.
    pub workspaces: Vec<RemovedWorkspace>,
    /// The conversations that targeted those workspaces, which now target
    /// their directories on the device directly.
    pub conversations: Vec<ConversationId>,
}

/// A workspace that went with its device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedWorkspace {
    pub id: WorkspaceId,
    pub name: String,
}

/// A `devices` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRecord {
    pub id: DeviceId,
    pub user: UserId,
    pub kind: DeviceKind,
    pub name: String,
    pub platform: RunnerPlatform,
    pub claimed_at: Timestamp,
    pub last_seen_at: Option<Timestamp>,
    /// What its runner last reported its artifact cache holds
    /// (`native-runtime.md` § Installed artifacts).
    pub installed: Vec<HostArtifact>,
}

const DEVICE_COLUMNS: &str =
    "id, user_id, kind, name, platform, claimed_at, last_seen_at, installed";

/// The name and platform of the one device a user's Cloud is.
const CLOUD_NAME: &str = "Cloud";
const CLOUD_PLATFORM: RunnerPlatform = RunnerPlatform::Linux;

impl ControlService {
    /// Stores a device the user paired, with the hash of the token its
    /// runner receives.
    pub async fn create_device(
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
                installed: Vec::new(),
            })
        })
        .await
    }

    pub async fn device(&self, id: DeviceId) -> Result<Option<DeviceRecord>, StorageError> {
        self.call(move |connection, _| one(connection, "id = ?1", id.as_str()))
            .await
    }

    /// The device whose current token has this hash.
    pub async fn device_by_token(
        &self,
        token: TokenHash,
    ) -> Result<Option<DeviceRecord>, StorageError> {
        self.call(move |connection, _| one(connection, "token_hash = ?1", token.as_str()))
            .await
    }

    /// The user's Cloud device, when its first use made it.
    pub async fn managed_device(&self, user: UserId) -> Result<Option<DeviceRecord>, StorageError> {
        self.call(move |connection, _| {
            one(
                connection,
                "kind = 'managed' AND user_id = ?1",
                user.as_str(),
            )
        })
        .await
    }

    /// The user's Cloud device, made on its first use. The partial unique
    /// index admits one per user, so concurrent first uses find the same
    /// one. Its token is issued when it boots.
    pub async fn managed_device_or_create(
        &self,
        user: UserId,
    ) -> Result<DeviceRecord, StorageError> {
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
    pub async fn paired_devices(&self, user: UserId) -> Result<Vec<DeviceRecord>, StorageError> {
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

    /// Deletes the device with its attachments to conversations and its
    /// workspaces, one transaction; its exposes go with it. The workspaces'
    /// conversations first move off them, each to its workspace's directory
    /// on the device as a direct device target, which no longer runs
    /// (`web-api.md` § Workspaces, devices, and attached hosts). A
    /// workspace is a pointer: no file goes.
    pub async fn delete_device(&self, device: DeviceId) -> Result<DeviceRemoval, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let conversations = {
                let mut statement = transaction.prepare_cached(
                    "UPDATE conversations SET target_kind = 'device', target_device_id = w.device_id,
                       target_path = w.path, target_workspace_id = NULL
                     FROM workspaces w
                     WHERE conversations.target_workspace_id = w.id AND w.device_id = ?1
                     RETURNING id",
                )?;
                let mut rows = statement.query([device.as_str()])?;
                let mut conversations = Vec::new();
                while let Some(row) = rows.next()? {
                    conversations.push(decode(
                        "conversations",
                        "id",
                        ConversationId::try_from(row.get::<_, String>("id")?),
                    )?);
                }
                conversations
            };
            let workspaces = {
                let mut statement = transaction.prepare_cached(
                    "DELETE FROM workspaces WHERE device_id = ?1 RETURNING id, name",
                )?;
                let mut rows = statement.query([device.as_str()])?;
                let mut workspaces = Vec::new();
                while let Some(row) = rows.next()? {
                    let id = decode(
                        "workspaces",
                        "id",
                        WorkspaceId::try_from(row.get::<_, String>("id")?),
                    )?;
                    workspaces.push(RemovedWorkspace {
                        id,
                        name: row.get("name")?,
                    });
                }
                workspaces
            };
            transaction.execute(
                "DELETE FROM conversation_hosts WHERE device_id = ?1",
                [device.as_str()],
            )?;
            transaction.execute("DELETE FROM devices WHERE id = ?1", [device.as_str()])?;
            transaction.commit()?;
            Ok(DeviceRemoval {
                workspaces,
                conversations,
            })
        })
        .await
    }

    /// Records what `device`'s runner reported its artifact cache holds.
    pub async fn set_device_installed(
        &self,
        device: DeviceId,
        installed: Vec<HostArtifact>,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "UPDATE devices SET installed = ?1 WHERE id = ?2",
                params![to_json(&installed), device.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// Records that the device's runner was connected just now.
    pub async fn touch_device_seen(&self, device: DeviceId) -> Result<(), StorageError> {
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
fn one(
    connection: &Connection,
    condition: &str,
    value: &str,
) -> Result<Option<DeviceRecord>, StorageError> {
    let mut statement = connection.prepare_cached(&format!(
        "SELECT {DEVICE_COLUMNS} FROM devices WHERE {condition}"
    ))?;
    statement
        .query_row([value], |row| Ok(device_row(row)))
        .optional()?
        .transpose()
}

/// A `devices` row, read from its columns in `DEVICE_COLUMNS`.
fn device_row(row: &Row<'_>) -> Result<DeviceRecord, StorageError> {
    let last_seen_at = match row.get::<_, Option<i64>>("last_seen_at")? {
        Some(millisecond) => Some(decode(
            "devices",
            "last_seen_at",
            Timestamp::from_millisecond(millisecond),
        )?),
        None => None,
    };
    Ok(DeviceRecord {
        id: decode(
            "devices",
            "id",
            DeviceId::try_from(row.get::<_, String>("id")?),
        )?,
        user: decode(
            "devices",
            "user_id",
            UserId::try_from(row.get::<_, String>("user_id")?),
        )?,
        kind: decode(
            "devices",
            "kind",
            row.get::<_, String>("kind")?.parse::<DeviceKind>(),
        )?,
        name: row.get("name")?,
        platform: decode(
            "devices",
            "platform",
            row.get::<_, String>("platform")?.parse::<RunnerPlatform>(),
        )?,
        claimed_at: instant(row, "devices", "claimed_at")?,
        last_seen_at,
        installed: json("devices", "installed", &row.get::<_, String>("installed")?)?,
    })
}
