//! Where a conversation runs, as the control store keeps it
//! (`sessions-and-targets.md` § Resolve a target): the Cloud's directory of
//! a conversation, a Host's home, or a workspace, and the one rule that
//! turns a destination, a directory on a Host or a project, into one of
//! them (`product.md` § Conversations and projects). Every move, a
//! conversation's creation, a restore whose project was deleted and the
//! upgrade's leftovers go through [`select`], in the transaction that
//! writes the selection, which makes the directory's workspace when the
//! user has none.

use demi_host_interface::HostPaths;
use demi_backend_remote_host::host_paths;
use demi_shared_types::Timestamp;
use demi_web_api_protocol::conversations::ConversationTarget;
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, params};

use super::StorageError;
use super::conversation_index::ExecutionTarget;
use super::devices::{DeviceRecord, cloud_device_in, device_in, managed_device_in};
use super::workspaces::{WorkspaceRecord, new_workspace_id, workspace_at};

/// The Cloud's home directory, before its runner reports one.
pub const CLOUD_HOME: &str = "/home/demi";

/// A conversation's selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// On the user's Cloud, in a conversation's directory: its own,
    /// `<home>/sessions/<id>`, without `path`, or after a Fork its
    /// source's, another directory directly under `<home>/sessions`.
    Cloud { path: Option<String> },
    /// A Host's home: a paired device's or the Cloud's.
    Home { device_id: DeviceId },
    /// A project, with the device and directory its workspace names, kept
    /// so a conversation whose project was deleted is restored to its
    /// directory.
    Workspace {
        workspace_id: WorkspaceId,
        device_id: DeviceId,
        path: String,
    },
    /// A directory on a paired device as a release before projects by
    /// directory kept it, until the device's runner first reports its home
    /// (`upgrades.md` § What crosses releases); and the directory of a
    /// conversation whose device was revoked, which no longer runs. No move
    /// selects it.
    Directory { device_id: DeviceId, path: String },
}

impl Selection {
    /// The selection as the web app reads a conversation's target, from
    /// `resolved`, what it resolves to: a project by its workspace, the
    /// Cloud's directory of a conversation as it was asked for, and a home
    /// or a directory as the Host and directory it is.
    pub fn shown(&self, resolved: &ExecutionTarget) -> ConversationTarget {
        match (self, resolved) {
            (Self::Cloud { path }, _) => ConversationTarget::Cloud { path: path.clone() },
            (Self::Workspace { workspace_id, .. }, _) => ConversationTarget::Workspace {
                workspace_id: workspace_id.clone(),
            },
            (_, ExecutionTarget::Cloud { path, .. }) => ConversationTarget::Cloud {
                path: Some(path.clone()),
            },
            (_, ExecutionTarget::Device { device_id, path } | ExecutionTarget::Workspace { device_id, path, .. }) => {
                ConversationTarget::Device {
                    device_id: device_id.clone(),
                    path: path.clone(),
                }
            }
        }
    }
}

/// Why a destination selects nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unselectable {
    /// The user has no workspace of the id.
    WorkspaceNotFound,
    /// The user paired no device of the id.
    DeviceNotFound,
    /// The device's runner never reported its home, so whether the
    /// directory is it is unknown.
    HomeUnknown,
}

/// A destination's selection, and the workspace selecting it made, if it
/// did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selected {
    pub selection: Selection,
    pub made: Option<WorkspaceRecord>,
}

impl Selected {
    fn kept(selection: Selection) -> Result<Self, Unselectable> {
        Ok(Self {
            selection,
            made: None,
        })
    }
}

/// The selection `to` names for `owner`'s conversation `conversation`, by
/// its directory: the Cloud's directory of a conversation, the Host's home,
/// or else the workspace of that device and directory, made now when the
/// user has none. A Cloud directory makes the user's Cloud device when it
/// was never made.
pub(crate) fn select(
    connection: &Connection,
    owner: &UserId,
    conversation: &ConversationId,
    to: &ConversationTarget,
    now: Timestamp,
) -> Result<Result<Selected, Unselectable>, StorageError> {
    match to {
        ConversationTarget::Cloud { path: None } => Ok(Selected::kept(Selection::Cloud { path: None })),
        ConversationTarget::Cloud { path: Some(path) } => {
            let cloud = cloud_device_in(connection, owner, now)?;
            select_directory(connection, conversation, &cloud, path, now)
        }
        ConversationTarget::Device { device_id, path } => {
            match device_in(connection, device_id)? {
                Some(device) if device.user == *owner && device.kind == DeviceKind::User => {
                    select_directory(connection, conversation, &device, path, now)
                }
                _ => Ok(Err(Unselectable::DeviceNotFound)),
            }
        }
        ConversationTarget::Workspace { workspace_id } => {
            let workspace = super::workspaces::workspace_in(connection, workspace_id)?;
            Ok(match workspace {
                Some(workspace) if workspace.user == *owner => Selected::kept(Selection::Workspace {
                    workspace_id: workspace.id,
                    device_id: workspace.device,
                    path: workspace.path,
                }),
                _ => Err(Unselectable::WorkspaceNotFound),
            })
        }
    }
}

/// The selection of `path` on `device` for `conversation`: [`select`]'s
/// rule for a directory.
pub(crate) fn select_directory(
    connection: &Connection,
    conversation: &ConversationId,
    device: &DeviceRecord,
    path: &str,
    now: Timestamp,
) -> Result<Result<Selected, Unselectable>, StorageError> {
    let paths = host_paths(device.platform);
    let path = directory(paths, path);
    let home = match device.kind {
        DeviceKind::Managed => {
            let home = device.home.as_deref().unwrap_or(CLOUD_HOME);
            let sessions = paths.join(home, "sessions");
            if paths.parent(&path).as_deref() == Some(sessions.as_str()) {
                let own = paths.join(&sessions, conversation.as_str());
                let path = (path != own).then_some(path);
                return Ok(Selected::kept(Selection::Cloud { path }));
            }
            home
        }
        DeviceKind::User => match device.home.as_deref() {
            Some(home) => home,
            None => return Ok(Err(Unselectable::HomeUnknown)),
        },
    };
    if path == directory(paths, home) {
        return Ok(Selected::kept(Selection::Home {
            device_id: device.id.clone(),
        }));
    }
    Ok(Ok(project_of(connection, device, &path, now)?))
}

/// The workspace of `path` on `device`, made now when its user has none:
/// named after the directory's last component, with *(2)*, *(3)* after it
/// when another of the device's projects has that name. A restore of a
/// conversation whose project was deleted selects it alone, since a
/// project's directory is never a home.
pub(crate) fn project_of(
    connection: &Connection,
    device: &DeviceRecord,
    path: &str,
    now: Timestamp,
) -> Result<Selected, StorageError> {
    let paths = host_paths(device.platform);
    let path = directory(paths, path);
    if let Some(workspace) = workspace_at(connection, &device.user, &device.id, &path)? {
        return Ok(Selected {
            selection: Selection::Workspace {
                workspace_id: workspace.id,
                device_id: workspace.device,
                path: workspace.path,
            },
            made: None,
        });
    }
    let base = match paths.file_name(&path) {
        "" => path.clone(),
        name => name.to_owned(),
    };
    let mut name = base.clone();
    let mut number = 1;
    while connection
        .query_row(
            "SELECT 1 FROM workspaces WHERE device_id = ?1 AND name = ?2",
            params![device.id.as_str(), name],
            |_| Ok(()),
        )
        .optional()?
        .is_some()
    {
        number += 1;
        name = format!("{base} ({number})");
    }
    let workspace = super::workspaces::insert_workspace(
        connection,
        new_workspace_id(),
        &device.user,
        &device.id,
        &path,
        &name,
        now,
    )?;
    Ok(Selected {
        selection: Selection::Workspace {
            workspace_id: workspace.id.clone(),
            device_id: workspace.device.clone(),
            path: workspace.path.clone(),
        },
        made: Some(workspace),
    })
}

/// `path` as a selection keeps it: without a separator after its last
/// component, so `/srv/app/` is the directory `/srv/app` is.
pub(crate) fn directory(paths: HostPaths, path: &str) -> String {
    paths.trimmed(path).to_owned()
}

/// What `owner`'s conversation `conversation` with `selection` resolves
/// to: its device and the directory its work starts in. The Cloud's home is
/// the one its runner last reported, `/home/demi` before it did.
pub(crate) fn execution_target(
    connection: &Connection,
    owner: &UserId,
    conversation: &ConversationId,
    selection: &Selection,
) -> Result<ExecutionTarget, StorageError> {
    Ok(match selection {
        Selection::Cloud { path } => {
            let cloud = managed_device_in(connection, owner)?;
            let path = match path {
                Some(path) => path.clone(),
                None => {
                    let home = cloud
                        .as_ref()
                        .and_then(|cloud| cloud.home.as_deref())
                        .unwrap_or(CLOUD_HOME);
                    HostPaths::Posix.join(&HostPaths::Posix.join(home, "sessions"), conversation.as_str())
                }
            };
            ExecutionTarget::Cloud {
                device_id: cloud.map(|cloud| cloud.id),
                path,
            }
        }
        Selection::Home { device_id } => {
            let device = device_in(connection, device_id)?.ok_or_else(|| StorageError::Corrupt {
                table: "conversations",
                column: "target_device_id",
                reason: format!("the home of device {device_id}, which is gone"),
            })?;
            match device.kind {
                DeviceKind::Managed => ExecutionTarget::Cloud {
                    path: device.home.unwrap_or_else(|| CLOUD_HOME.to_owned()),
                    device_id: Some(device.id),
                },
                DeviceKind::User => ExecutionTarget::Device {
                    path: device.home.ok_or_else(|| StorageError::Corrupt {
                        table: "conversations",
                        column: "target_kind",
                        reason: format!("the home of device {device_id}, which never reported one"),
                    })?,
                    device_id: device.id,
                },
            }
        }
        Selection::Workspace {
            workspace_id,
            device_id,
            path,
        } => ExecutionTarget::Workspace {
            workspace_id: workspace_id.clone(),
            device_id: device_id.clone(),
            path: path.clone(),
        },
        Selection::Directory { device_id, path } => ExecutionTarget::Device {
            device_id: device_id.clone(),
            path: path.clone(),
        },
    })
}
