//! Device records (`storage.md` § Control records): who owns a device, how
//! it came to be, and the hash of its current token, which finds the device
//! of a runner's hello or pipe request through one index lookup, and what
//! its runner last reported its artifact cache holds. Whether a device is
//! online is its runner connection's, not a record's.

use demi_runner_protocol::wire::{HostArtifact, OperatingSystem, RunnerPlatform};
use demi_shared_types::Timestamp;
use demi_web_api_protocol::devices::{DeviceKind, DeviceLogo, DeviceRoute};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::accounts::TokenHash;
use super::columns::{decode, instant, json, to_json};
use super::control::ControlService;
use super::conversation_index::raise_hosts_revision;

/// What a device's deletion removed with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRemoval {
    /// The workspaces whose device it was.
    pub workspaces: Vec<RemovedWorkspace>,
    /// The conversations the device's removal changed: those that targeted
    /// those workspaces, which now target their directories on the device
    /// directly, and those it was attached to, each once.
    pub conversations: Vec<ConversationId>,
}

/// A device as a change left it, and the conversations whose execution
/// context its new name changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedDevice {
    pub device: DeviceRecord,
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
    /// The operating system its runner last reported; none before its
    /// runner first connected.
    pub os: Option<OperatingSystem>,
    /// The runner release its runner last reported, such as `0.1.16`; none
    /// before its runner first connected.
    pub runner_version: Option<String>,
    /// The instance its runner last named in its hello: a number each start
    /// of the runner draws (`runner.md` § Command lifetime); none before its
    /// runner first connected.
    pub runner_instance: Option<u64>,
    /// When a Cloud's latest boot completed, as its runner connected: its
    /// lifetime cap counts from it (`managed-hosts.md` § Lifecycle and
    /// capacity). None for a paired device and a Cloud that never booted.
    pub cloud_started_at: Option<Timestamp>,
    /// How pages reach it (`direct-channel.md` § Choosing the path).
    pub route: DeviceRoute,
    /// A paired device's number among its user's devices of its logo
    /// (`product.md` § Conversations and projects); none before its runner
    /// first reported its system, and for a Cloud.
    pub mark_number: Option<u32>,
    /// Which managed machine a managed device is; none for a paired device.
    pub machine: Option<ManagedMachine>,
}

impl DeviceRecord {
    /// Whether the device is one of `user`'s Hosts: theirs, and not the
    /// Claude Code machine, which is in no user's Hosts though its record
    /// is the master's (`managed-hosts.md` § The Claude Code machine).
    pub fn is_host_of(&self, user: &UserId) -> bool {
        self.user == *user && self.machine != Some(ManagedMachine::ClaudeCode)
    }
}

/// A managed device's machine (`managed-hosts.md` § The Claude Code
/// machine): each user's Cloud, or the Claude Code machine, whose record
/// belongs to the instance's master and which is in no user's Hosts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ManagedMachine {
    Cloud,
    ClaudeCode,
}

impl ManagedMachine {
    /// Its name in the `devices.machine` column.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cloud => "cloud",
            Self::ClaudeCode => "claude_code",
        }
    }

    fn parse(text: &str) -> Result<Self, String> {
        match text {
            "cloud" => Ok(Self::Cloud),
            "claude_code" => Ok(Self::ClaudeCode),
            other => Err(format!("unknown managed machine {other}")),
        }
    }

    /// The name its device record carries.
    fn name(self) -> &'static str {
        match self {
            Self::Cloud => CLOUD_NAME,
            Self::ClaudeCode => CLAUDE_CODE_NAME,
        }
    }
}

/// A paired device's system: its platform, and the operating system its
/// runner last reported, none before its runner first connected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedSystem {
    pub platform: RunnerPlatform,
    pub os: Option<OperatingSystem>,
}

/// What a change of a device sets: each field that is present.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceChange {
    pub name: Option<String>,
    pub route: Option<DeviceRoute>,
}

const DEVICE_COLUMNS: &str =
    "id, user_id, kind, name, platform, claimed_at, last_seen_at, installed, os, runner_version, runner_instance, cloud_started_at, route, mark_number, machine";

/// The name and platform of the one device a user's Cloud is.
pub const CLOUD_NAME: &str = "Cloud";
/// The name of the Claude Code machine's record, which no page shows.
const CLAUDE_CODE_NAME: &str = "Claude Code";
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
                os: None,
                runner_version: None,
                runner_instance: None,
                cloud_started_at: None,
                route: DeviceRoute::Automatic,
                mark_number: None,
                machine: None,
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
        self.managed_machine(user, ManagedMachine::Cloud).await
    }

    /// The user's Cloud device, made on its first use.
    pub async fn managed_device_or_create(
        &self,
        user: UserId,
    ) -> Result<DeviceRecord, StorageError> {
        self.managed_machine_or_create(user, ManagedMachine::Cloud).await
    }

    /// The user's managed device of `machine`, when its first use made it.
    pub async fn managed_machine(
        &self,
        user: UserId,
        machine: ManagedMachine,
    ) -> Result<Option<DeviceRecord>, StorageError> {
        self.call(move |connection, _| {
            one_of(
                connection,
                "kind = 'managed' AND user_id = ?1 AND machine = ?2",
                [user.as_str(), machine.as_str()],
            )
        })
        .await
    }

    /// The user's managed device of `machine`, made on its first use: the
    /// Cloud of any user, the Claude Code machine of the master. The partial
    /// unique index admits one of each per user, so concurrent first uses
    /// find the same one. Its token is issued when it boots.
    pub async fn managed_machine_or_create(
        &self,
        user: UserId,
        machine: ManagedMachine,
    ) -> Result<DeviceRecord, StorageError> {
        let id = DeviceId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "INSERT INTO devices (id, user_id, kind, name, platform, token_hash, claimed_at, last_seen_at, machine)
                 VALUES (?1, ?2, 'managed', ?3, ?4, NULL, ?5, NULL, ?6)
                 ON CONFLICT DO NOTHING",
                params![
                    id.as_str(),
                    user.as_str(),
                    machine.name(),
                    CLOUD_PLATFORM.to_string(),
                    now.as_millisecond(),
                    machine.as_str()
                ],
            )?;
            let device = one_of(
                &transaction,
                "kind = 'managed' AND user_id = ?1 AND machine = ?2",
                [user.as_str(), machine.as_str()],
            )?;
            transaction.commit()?;
            device.ok_or(StorageError::Corrupt {
                table: "devices",
                column: "machine",
                reason: "the user's managed device was neither found nor made".into(),
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

    /// The systems of every user's paired devices, each once: the systems
    /// whose runners this backend serves, and whose targets the commands it
    /// stores ahead are for.
    pub async fn paired_systems(&self) -> Result<Vec<PairedSystem>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection
                .prepare_cached("SELECT DISTINCT platform, os FROM devices WHERE kind = 'user'")?;
            let mut rows = statement.query([])?;
            let mut systems = Vec::new();
            while let Some(row) = rows.next()? {
                systems.push(PairedSystem {
                    platform: decode(
                        "devices",
                        "platform",
                        row.get::<_, String>("platform")?.parse::<RunnerPlatform>(),
                    )?,
                    os: match row.get::<_, Option<String>>("os")? {
                        Some(text) => Some(json("devices", "os", &text)?),
                        None => None,
                    },
                });
            }
            Ok(systems)
        })
        .await
    }

    /// Deletes the device with its attachments to conversations and its
    /// workspaces, one transaction. The workspaces' conversations first move
    /// off them, each to its workspace's directory
    /// on the device as a direct device target, which no longer runs
    /// (`web-api.md` § Workspaces, devices, and attached hosts). A
    /// workspace is a pointer: no file goes.
    pub async fn delete_device(&self, device: DeviceId) -> Result<DeviceRemoval, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let mut conversations = {
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
            let detached = {
                let mut statement = transaction.prepare_cached(
                    "DELETE FROM conversation_hosts WHERE device_id = ?1 RETURNING conversation_id",
                )?;
                let mut rows = statement.query([device.as_str()])?;
                let mut detached = Vec::new();
                while let Some(row) = rows.next()? {
                    detached.push(decode(
                        "conversation_hosts",
                        "conversation_id",
                        ConversationId::try_from(row.get::<_, String>("conversation_id")?),
                    )?);
                }
                detached
            };
            for conversation in detached {
                raise_hosts_revision(&transaction, &conversation)?;
                if !conversations.contains(&conversation) {
                    conversations.push(conversation);
                }
            }
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

    /// Records the operating system, the runner release and the instance
    /// `device`'s runner reported in its hello, and advances the execution
    /// context of each conversation whose context the report changes
    /// (`sessions-and-targets.md` § Resolve a target): when the system
    /// changed, of those whose primary Host the device is; when
    /// `home_changed`, the home its contexts named before differing from
    /// the one reported, of those whose context names a directory under
    /// that home: a Cloud conversation working in its session directory,
    /// and one the device is attached to without a directory of its own.
    /// A paired device whose mark has no number yet, or whose logo the
    /// report changes, takes one more than the highest number among its
    /// user's other devices of its new logo (`product.md` § Conversations
    /// and projects).
    pub async fn set_device_runner(
        &self,
        device: DeviceId,
        os: OperatingSystem,
        runner_version: String,
        runner_instance: u64,
        home_changed: bool,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let Some(record) = one(&transaction, "id = ?1", device.as_str())? else {
                return Ok(());
            };
            let logo = DeviceLogo::of(record.platform, Some(&os));
            let mark_number = match record.kind {
                DeviceKind::Managed => None,
                DeviceKind::User
                    if record.mark_number.is_some()
                        && DeviceLogo::of(record.platform, record.os.as_ref()) == logo =>
                {
                    record.mark_number
                }
                DeviceKind::User => Some(next_mark_number(&transaction, &record, logo)?),
            };
            transaction.execute(
                "UPDATE devices SET os = ?1, runner_version = ?2, runner_instance = ?3, mark_number = ?4
                 WHERE id = ?5",
                params![
                    to_json(&os),
                    runner_version,
                    runner_instance.cast_signed(),
                    mark_number,
                    device.as_str()
                ],
            )?;
            let system_changed = record.os.as_ref() != Some(&os);
            if system_changed || home_changed {
                // A Cloud conversation names no device: the Cloud is its
                // owner's managed one.
                transaction.execute(
                    "UPDATE conversations SET context_version = context_version + 1
                     WHERE id IN (
                       SELECT conversation.id FROM conversations conversation
                         JOIN devices device ON device.id = ?1
                       WHERE (?2 AND (
                           conversation.target_device_id = device.id
                           OR conversation.target_workspace_id IN
                             (SELECT id FROM workspaces WHERE device_id = device.id)
                           OR (conversation.target_kind = 'cloud' AND device.kind = 'managed'
                             AND device.machine = 'cloud' AND conversation.user_id = device.user_id)))
                         OR (?3 AND (
                           (conversation.target_kind = 'cloud' AND conversation.target_path IS NULL
                             AND device.kind = 'managed' AND device.machine = 'cloud'
                             AND conversation.user_id = device.user_id)
                           OR EXISTS (SELECT 1 FROM conversation_hosts host
                             WHERE host.conversation_id = conversation.id
                               AND host.device_id = device.id AND host.cwd IS NULL))))",
                    params![device.as_str(), system_changed, home_changed],
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    /// Records that `device`, a Cloud, completed a boot now; answers when.
    pub async fn record_cloud_boot(&self, device: DeviceId) -> Result<Timestamp, StorageError> {
        self.call(move |connection, now| {
            connection.execute(
                "UPDATE devices SET cloud_started_at = ?1 WHERE id = ?2",
                params![now.as_millisecond(), device.as_str()],
            )?;
            Ok(now)
        })
        .await
    }

    /// Sets what `change` names of `device`; none when there is no such
    /// device. A conversation that reaches the device, as an attached host
    /// or as its primary one, calls it by its name, which its attached
    /// hosts' names count, so a new name advances the execution context and
    /// the attached hosts' revision of each; the answer names those
    /// conversations.
    pub async fn change_device(
        &self,
        device: DeviceId,
        change: DeviceChange,
    ) -> Result<Option<ChangedDevice>, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let Some(before) = one(&transaction, "id = ?1", device.as_str())? else {
                return Ok(None);
            };
            let changed = transaction
                .prepare_cached(&format!(
                    "UPDATE devices SET name = coalesce(?1, name), route = coalesce(?2, route)
                     WHERE id = ?3 RETURNING {DEVICE_COLUMNS}"
                ))?
                .query_row(
                    params![change.name, change.route.map(|route| route.to_string()), device.as_str()],
                    |row| Ok(device_row(row)),
                )??;
            let conversations = if changed.name == before.name {
                Vec::new()
            } else {
                transaction
                    .prepare_cached(
                        "UPDATE conversations SET context_version = context_version + 1,
                           hosts_revision = hosts_revision + 1
                         WHERE id IN (SELECT conversation_id FROM conversation_hosts WHERE device_id = ?1)
                           OR target_device_id = ?1
                           OR target_workspace_id IN (SELECT id FROM workspaces WHERE device_id = ?1)
                         RETURNING id",
                    )?
                    .query_map([device.as_str()], |row| row.get::<_, String>(0))?
                    .map(|id| decode("conversations", "id", ConversationId::try_from(id?.as_str())))
                    .collect::<Result<Vec<_>, _>>()?
            };
            transaction.commit()?;
            Ok(Some(ChangedDevice {
                device: changed,
                conversations,
            }))
        })
        .await
    }

    /// Records that the backend's shutdown ended the connections of
    /// `devices` (`sessions-and-targets.md` § Recovery and persistence).
    pub async fn set_devices_ended_by_shutdown(
        &self,
        devices: Vec<DeviceId>,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            {
                let mut statement = transaction
                    .prepare_cached("UPDATE devices SET ended_by_shutdown = 1 WHERE id = ?1")?;
                for device in &devices {
                    statement.execute([device.as_str()])?;
                }
            }
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    /// The devices whose connections the backend's last shutdown ended, for
    /// the start that follows it, which takes them: none is recorded after.
    pub async fn take_devices_ended_by_shutdown(&self) -> Result<Vec<DeviceId>, StorageError> {
        self.call(|connection, _| {
            let mut statement = connection.prepare_cached(
                "UPDATE devices SET ended_by_shutdown = 0 WHERE ended_by_shutdown = 1 RETURNING id",
            )?;
            let ids = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            ids.into_iter()
                .map(|id| decode("devices", "id", DeviceId::try_from(id)))
                .collect()
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

/// One more than the highest mark number among the other paired devices of
/// `device`'s user whose logo is `logo`; 1 when none has a number.
fn next_mark_number(
    connection: &Connection,
    device: &DeviceRecord,
    logo: DeviceLogo,
) -> Result<u32, StorageError> {
    let mut statement = connection.prepare_cached(&format!(
        "SELECT {DEVICE_COLUMNS} FROM devices
         WHERE user_id = ?1 AND kind = 'user' AND id != ?2 AND mark_number IS NOT NULL"
    ))?;
    let mut rows = statement.query(params![device.user.as_str(), device.id.as_str()])?;
    let mut highest = 0;
    while let Some(row) = rows.next()? {
        let other = device_row(row)?;
        if DeviceLogo::of(other.platform, other.os.as_ref()) == logo {
            highest = highest.max(other.mark_number.unwrap_or(0));
        }
    }
    Ok(highest + 1)
}

/// The one device `condition` selects with `value`.
fn one(
    connection: &Connection,
    condition: &str,
    value: &str,
) -> Result<Option<DeviceRecord>, StorageError> {
    one_of(connection, condition, [value])
}

/// The device `condition` finds with the values `values`.
fn one_of(
    connection: &Connection,
    condition: &str,
    values: impl rusqlite::Params,
) -> Result<Option<DeviceRecord>, StorageError> {
    let mut statement = connection.prepare_cached(&format!(
        "SELECT {DEVICE_COLUMNS} FROM devices WHERE {condition}"
    ))?;
    statement
        .query_row(values, |row| Ok(device_row(row)))
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
        os: match row.get::<_, Option<String>>("os")? {
            Some(text) => Some(json("devices", "os", &text)?),
            None => None,
        },
        runner_version: row.get("runner_version")?,
        runner_instance: row
            .get::<_, Option<i64>>("runner_instance")?
            .map(i64::cast_unsigned),
        cloud_started_at: match row.get::<_, Option<i64>>("cloud_started_at")? {
            Some(millisecond) => Some(decode(
                "devices",
                "cloud_started_at",
                Timestamp::from_millisecond(millisecond),
            )?),
            None => None,
        },
        route: decode(
            "devices",
            "route",
            row.get::<_, String>("route")?.parse::<DeviceRoute>(),
        )?,
        mark_number: match row.get::<_, Option<i64>>("mark_number")? {
            Some(number) => Some(decode("devices", "mark_number", u32::try_from(number))?),
            None => None,
        },
        machine: match row.get::<_, Option<String>>("machine")? {
            Some(text) => Some(decode("devices", "machine", ManagedMachine::parse(&text))?),
            None => None,
        },
    })
}
