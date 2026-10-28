//! Where a conversation runs, as the control store keeps it: the Cloud's
//! directory of a conversation, a Host's home, or a workspace.
//!
//! One rule turns a destination, a directory on a Host or a project, into one of
//! them. Every move, a conversation's creation, a restore whose project was
//! deleted and the upgrade's leftovers go through [`select`], in the transaction
//! that writes the selection, which makes the directory's workspace when the user
//! has none.
//!
//! # Directories
//!
//! A move names a directory, never a selection kind, so no conversation runs
//! outside a project in a project's directory, or in a project in its Host's home.
//!
//! A directory is known by its canonical path on its Host, as the Host resolves it
//! when the move checks the directory: links followed, no trailing separator, the
//! spelling the file system keeps (on Windows `C:/Users/alex/` and
//! `c:\users\alex` are the one directory `C:\Users\alex`). The runner reports its
//! home the same way, so two spellings of a directory are never two projects and a
//! home reached through a link is still the home.
//!
//! # Homes
//!
//! - A move to a device whose home was never reported is refused: a move needs the
//!   device online, which reports it.
//! - The home is kept with the device's record, so which directory is its home is
//!   known while it is offline. The Cloud's is `/home/demi` until its runner first
//!   reports.
//! - When a report changes what an execution context said, the context advances,
//!   so a node's next inference names the directory its shells really start in and
//!   never hands on a path that does not exist on its Host.
//!
//! # Workspaces
//!
//! A workspace is a `(user, device, path)` record, one per device and
//! directory, named by the directory. It groups conversations and owns no
//! sandbox: it restricts no filesystem access. A selection of a deleted one's
//! directory, or of a worktree it lists, brings it back.
//!
//! # Worktrees
//!
//! A linked worktree of a project's repository is never a project of its own:
//! a conversation that works in one is in the project of the repository's main
//! worktree, with the worktree, which it works in in place of the project's
//! directory. For example, `~/app` is a project, and `git worktree add
//! ../app-2` made the worktree `app-2` of its repository; a conversation moved
//! to `~/app-2` is in `app`, with the worktree `app-2`.
//!
//! - The project's Host reports the worktrees of each project's repository
//!   (`crates/runner-host/src/worktrees.rs`), before any message that could
//!   follow a change of them, and each project keeps them as last reported, so
//!   a move selects by them in its own transaction, with the Host away.
//! - A directory selects, in order: the project of that directory; the
//!   project that lists it as a worktree, with the worktree; a project made for
//!   it.
//! - A report that lists a worktree whose directory is a project of its own,
//!   made before the report or before worktrees were known, moves that
//!   project's conversations into the repository's project, with the
//!   worktree, and deletes it, as a second spelling of a project's directory
//!   goes into the project (`ControlService::name_directory`).

use demi_host_interface::HostPaths;
use demi_runner_client::host_paths;
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
    /// directory, and the linked worktree of the project's repository it
    /// works in, if it does (§ Worktrees).
    Workspace {
        workspace_id: WorkspaceId,
        device_id: DeviceId,
        path: String,
        worktree: Option<Worktree>,
    },
    /// A directory on a paired device as a release before projects by
    /// directory kept it, until the device's runner first reports its home
    /// (`crates/server/src/main.rs`); and the directory of a conversation
    /// whose device was revoked, which no longer runs. No move selects it.
    Directory { device_id: DeviceId, path: String },
}

/// A linked worktree of a project's repository (§ Worktrees).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    /// The name git gave it: its directory's under `.git/worktrees/`.
    pub name: String,
    /// Its directory, as its Host reported it.
    pub path: String,
}

impl Selection {
    /// The directory the selection's work starts in, where it names one: a
    /// project's worktree's in place of the project's.
    pub fn worktree(&self) -> Option<&Worktree> {
        match self {
            Self::Workspace { worktree, .. } => worktree.as_ref(),
            _ => None,
        }
    }

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

/// The selection `to` names for `owner`'s conversation `conversation`, by its
/// directory: the Cloud's directory of a conversation, the Host's home, or else
/// the workspace of that device and directory, made now when the user has none.
///
/// A Cloud directory makes the user's Cloud device when it was never made.
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
                Some(workspace) if workspace.user == *owner && !workspace.deleted => Selected::kept(Selection::Workspace {
                    workspace_id: workspace.id,
                    device_id: workspace.device,
                    path: workspace.path,
                    worktree: None,
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
    if workspace_at(connection, &device.user, &device.id, &path)?.is_none()
        && let Some(selection) = worktree_at(connection, device, &path)?
    {
        let made = match &selection {
            Selection::Workspace { workspace_id, .. } if super::workspaces::revive(connection, workspace_id)? => {
                super::workspaces::workspace_in(connection, workspace_id)?
            }
            _ => None,
        };
        return Ok(Ok(Selected { selection, made }));
    }
    Ok(Ok(project_of(connection, device, &path, now)?))
}

/// The selection of the linked worktree at `path` on `device`, in the project
/// that lists it (§ Worktrees); none when no project of the device's user
/// does.
pub(crate) fn worktree_at(
    connection: &Connection,
    device: &DeviceRecord,
    path: &str,
) -> Result<Option<Selection>, StorageError> {
    Ok(connection
        .prepare_cached(
            "SELECT workspace.id, workspace.path, worktree.name, worktree.path
             FROM workspace_worktrees worktree JOIN workspaces workspace ON workspace.id = worktree.workspace_id
             WHERE workspace.user_id = ?1 AND workspace.device_id = ?2 AND worktree.path = ?3
             ORDER BY workspace.id LIMIT 1",
        )?
        .query_row(params![device.user.as_str(), device.id.as_str(), path], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))
        })
        .optional()?
        .map(|(workspace, project, name, path)| -> Result<Selection, StorageError> {
            Ok(Selection::Workspace {
                workspace_id: super::columns::decode("workspace_worktrees", "workspace_id", WorkspaceId::try_from(workspace))?,
                device_id: device.id.clone(),
                path: project,
                worktree: Some(Worktree { name, path }),
            })
        })
        .transpose()?)
}

/// The workspace of `path` on `device`, made now when its user has none, and
/// brought back when the user deleted it.
///
/// A restore of a conversation whose project was deleted selects it alone, since a
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
        let made = if super::workspaces::revive(connection, &workspace.id)? {
            super::workspaces::workspace_in(connection, &workspace.id)?
        } else {
            None
        };
        return Ok(Selected {
            selection: Selection::Workspace {
                workspace_id: workspace.id,
                device_id: workspace.device,
                path: workspace.path,
                worktree: None,
            },
            made,
        });
    }
    let workspace = super::workspaces::insert_workspace(connection, new_workspace_id(), &device.user, &device.id, &path, now)?;
    Ok(Selected {
        selection: Selection::Workspace {
            workspace_id: workspace.id.clone(),
            device_id: workspace.device.clone(),
            path: workspace.path.clone(),
            worktree: None,
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
            worktree,
        } => ExecutionTarget::Workspace {
            workspace_id: workspace_id.clone(),
            device_id: device_id.clone(),
            path: worktree.as_ref().map_or(path, |worktree| &worktree.path).clone(),
        },
        Selection::Directory { device_id, path } => ExecutionTarget::Device {
            device_id: device_id.clone(),
            path: path.clone(),
        },
    })
}
