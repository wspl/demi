//! Workspace records: a named directory on one of the user's devices, which
//! conversations can target.
//!
//! - A workspace is a pointer: it owns no files, and the user has one per device
//!   and directory.
//! - A new workspace joins the user's others at the end of their order.
//! - It keeps the model settings and the workflow the project's next conversation
//!   starts with.

use demi_shared_types::{Timestamp, WorkflowChoice};
use demi_web_api_protocol::conversations::ModelSettings;
use demi_backend_remote_host::host_paths;
use demi_host_interface::HostPaths;
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId, WorkspaceId};
use demi_web_api_protocol::workspaces::WorkspaceDto;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{decode, instant, json, to_json, workflow_choice, workflow_columns};
use super::control::ControlService;
use super::conversation_index::archive_in;
use super::devices::device_in;
use super::selection::{CLOUD_HOME, directory};

const WORKSPACE_COLUMNS: &str =
    "id, user_id, device_id, path, name, created_at, last_model, last_workflow_kind, last_workflow_id";

/// A new workspace's id, which the backend assigns.
pub fn new_workspace_id() -> WorkspaceId {
    WorkspaceId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty")
}

/// A `workspaces` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRecord {
    pub id: WorkspaceId,
    pub user: UserId,
    pub device: DeviceId,
    pub path: String,
    pub name: String,
    pub created_at: Timestamp,
    /// The model settings last chosen in the project; none before the first
    /// choice.
    pub last_model: Option<ModelSettings>,
    /// The Workflow menu choice last made in the project; none before the
    /// first.
    pub last_workflow: Option<WorkflowChoice>,
}

/// A change of a workspace: each field it holds replaces the stored one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceChange {
    pub name: Option<String>,
    pub last_model: Option<ModelSettings>,
    pub last_workflow: Option<WorkflowChoice>,
}

impl WorkspaceRecord {
    /// The workspace as the page lists it.
    pub fn dto(self) -> WorkspaceDto {
        WorkspaceDto {
            id: self.id,
            device_id: self.device,
            path: self.path,
            name: self.name,
            created_at: self.created_at,
            last_model: self.last_model,
            last_workflow: self.last_workflow,
        }
    }
}

/// What a workspace's deletion did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceDeletion {
    Deleted,
    /// The user has no workspace of the id.
    Missing,
    /// The conversations in it are not the ones the deletion held: one
    /// moved in or was created meanwhile. Nothing changed.
    Changed,
}

/// What a workspace's creation did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceCreation {
    Created(WorkspaceRecord),
    /// The user has a workspace of the device and directory already.
    Exists(WorkspaceRecord),
    /// The device is not the user's.
    DeviceNotFound,
    /// The directory is the device's home, which holds the conversations
    /// outside projects.
    Home,
}

impl ControlService {
    pub async fn workspace(
        &self,
        id: WorkspaceId,
    ) -> Result<Option<WorkspaceRecord>, StorageError> {
        self.call(move |connection, _| workspace_by_id(connection, &id))
            .await
    }

    /// The user's workspaces, in their order.
    pub async fn workspaces(&self, user: UserId) -> Result<Vec<WorkspaceRecord>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE user_id = ?1 ORDER BY sort_order, id"
            ))?;
            let mut rows = statement.query([user.as_str()])?;
            let mut workspaces = Vec::new();
            while let Some(row) = rows.next()? {
                workspaces.push(workspace_row(row)?);
            }
            Ok(workspaces)
        })
        .await
    }

    /// The new workspace `id` at `path` on the user's `device`, after the user's
    /// others.
    ///
    /// It is made unless the device is not the user's, the directory is its home,
    /// or the user has a workspace of it already, which it answers. The checks and
    /// the write are one transaction, so a revocation cannot slip between them.
    pub async fn create_workspace(
        &self,
        id: WorkspaceId,
        user: UserId,
        device: DeviceId,
        path: String,
        name: String,
    ) -> Result<WorkspaceCreation, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let Some(record) = device_in(&transaction, &device)?.filter(|record| record.user == user) else {
                return Ok(WorkspaceCreation::DeviceNotFound);
            };
            let paths = host_paths(record.platform);
            let path = directory(paths, &path);
            let home = match record.kind {
                DeviceKind::Managed => Some(record.home.as_deref().unwrap_or(CLOUD_HOME)),
                DeviceKind::User => record.home.as_deref(),
            };
            if home.is_some_and(|home| directory(paths, home) == path) {
                return Ok(WorkspaceCreation::Home);
            }
            if let Some(existing) = workspace_at(&transaction, &user, &device, &path)? {
                return Ok(WorkspaceCreation::Exists(existing));
            }
            let workspace = insert_workspace(&transaction, id, &user, &device, &path, &name, now)?;
            transaction.commit()?;
            Ok(WorkspaceCreation::Created(workspace))
        })
        .await
    }

    /// Applies `change` to the user's workspace `id` and answers it as it
    /// now is; `None` when the user has no workspace of the id.
    pub async fn change_workspace(
        &self,
        user: UserId,
        id: WorkspaceId,
        change: WorkspaceChange,
    ) -> Result<Option<WorkspaceRecord>, StorageError> {
        self.call(move |connection, _| {
            let (kind, workflow) = match &change.last_workflow {
                // A new conversation is in no worktree, so the project's
                // last choice keeps the workflow alone.
                Some(choice) => {
                    let (kind, workflow, _worktree) = workflow_columns(choice);
                    (Some(kind), workflow)
                }
                None => (None, None),
            };
            let mut statement = connection.prepare_cached(&format!(
                "UPDATE workspaces SET
                   name = COALESCE(?1, name),
                   last_model = COALESCE(?2, last_model),
                   last_workflow_kind = COALESCE(?3, last_workflow_kind),
                   last_workflow_id = CASE WHEN ?3 IS NULL THEN last_workflow_id ELSE ?4 END
                 WHERE id = ?5 AND user_id = ?6 RETURNING {WORKSPACE_COLUMNS}"
            ))?;
            let mut rows = statement.query(params![
                change.name,
                change.last_model.as_ref().map(to_json),
                kind,
                workflow,
                id.as_str(),
                user.as_str()
            ])?;
            rows.next()?.map(workspace_row).transpose()
        })
        .await
    }

    /// The user's conversations in workspace `id` that are not archived.
    pub async fn workspace_conversations(
        &self,
        user: UserId,
        id: WorkspaceId,
    ) -> Result<Vec<ConversationId>, StorageError> {
        self.call(move |connection, _| workspace_conversations(connection, &user, &id))
            .await
    }

    /// Deletes the user's workspace `id` and archives `conversations`, the
    /// ones in it, as one transaction; when they are no longer all the
    /// unarchived ones in it, nothing changes. An archived conversation in it
    /// keeps its directory, which its restore selects again.
    pub async fn delete_workspace(
        &self,
        user: UserId,
        id: WorkspaceId,
        conversations: Vec<ConversationId>,
    ) -> Result<WorkspaceDeletion, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let found = transaction
                .query_row(
                    "SELECT 1 FROM workspaces WHERE id = ?1 AND user_id = ?2",
                    params![id.as_str(), user.as_str()],
                    |_| Ok(()),
                )
                .optional()?;
            if found.is_none() {
                return Ok(WorkspaceDeletion::Missing);
            }
            let mut inside = workspace_conversations(&transaction, &user, &id)?;
            let mut held = conversations;
            inside.sort();
            held.sort();
            if inside != held {
                return Ok(WorkspaceDeletion::Changed);
            }
            for conversation in &held {
                archive_in(&transaction, conversation)?;
            }
            transaction.execute("DELETE FROM workspaces WHERE id = ?1", [id.as_str()])?;
            transaction.commit()?;
            Ok(WorkspaceDeletion::Deleted)
        })
        .await
    }
}

/// What a Host's first naming of a directory changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirectoryNamed {
    /// The conversations whose directory is now spelled otherwise, which
    /// their pages and agents learn.
    pub conversations: Vec<ConversationId>,
    /// The projects changed: one renamed, or one that went into another.
    pub projects: bool,
}

/// The directory a conversation's target stores as typed, which its Host
/// names on its first use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypedDirectory {
    /// A project's, whose conversations all share it.
    Workspace(WorkspaceId),
    /// A directory of the Cloud that is the conversation's own selection.
    Cloud(String),
}

impl ControlService {
    /// Replaces the directory `typed` with `canonical`, as its Host names it.
    ///
    /// - A project whose directory is another's spelling takes that spelling:
    ///   the project is renamed in place, with the conversations in it.
    /// - A project whose canonical directory is another project's goes into
    ///   that project, its conversations with it, since one directory is one
    ///   project.
    /// - A conversation of the Cloud takes the spelling for its directory.
    ///
    /// A directory that is the Host's home stays as typed, which is no
    /// project's. Answers what changed.
    pub async fn name_directory(
        &self,
        conversation: ConversationId,
        typed: TypedDirectory,
        canonical: String,
    ) -> Result<DirectoryNamed, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let named = match typed {
                TypedDirectory::Cloud(path) => {
                    let Some(record) = super::conversation_index::conversation_by_id(&transaction, &conversation)? else {
                        return Ok(DirectoryNamed::default());
                    };
                    let current = matches!(&record.target, super::Selection::Cloud { path: Some(current) } if *current == path);
                    let canonical = HostPaths::Posix.trimmed(&canonical).to_owned();
                    if !current || canonical == path {
                        return Ok(DirectoryNamed::default());
                    }
                    super::devices::set_selection(
                        &transaction,
                        &conversation,
                        &super::Selection::Cloud { path: Some(canonical) },
                    )?;
                    super::conversation_index::advance_context(&transaction, &conversation)?;
                    DirectoryNamed { conversations: vec![conversation], projects: false }
                }
                TypedDirectory::Workspace(id) => {
                    let Some(project) = workspace_by_id(&transaction, &id)? else {
                        return Ok(DirectoryNamed::default());
                    };
                    let Some(device) = device_in(&transaction, &project.device)? else {
                        return Ok(DirectoryNamed::default());
                    };
                    let paths = host_paths(device.platform);
                    let canonical = paths.trimmed(&canonical).to_owned();
                    let home = device.home.as_deref().map(|home| paths.trimmed(home));
                    if canonical == project.path || home == Some(canonical.as_str()) {
                        return Ok(DirectoryNamed::default());
                    }
                    let inside: Vec<ConversationId> = transaction
                        .prepare_cached("SELECT id FROM conversations WHERE target_workspace_id = ?1")?
                        .query_map([id.as_str()], |row| row.get::<_, String>(0))?
                        .map(|id| decode("conversations", "id", ConversationId::try_from(id?.as_str())))
                        .collect::<Result<_, _>>()?;
                    match workspace_at(&transaction, &project.user, &project.device, &canonical)? {
                        Some(other) if other.id != project.id => {
                            // Each keeps the worktree it works in.
                            transaction.execute(
                                "UPDATE conversations SET target_workspace_id = ?2, target_path = ?3 WHERE target_workspace_id = ?1",
                                params![id.as_str(), other.id.as_str(), other.path],
                            )?;
                            for member in &inside {
                                super::conversation_index::advance_context(&transaction, member)?;
                            }
                            transaction.execute("DELETE FROM workspaces WHERE id = ?1", [id.as_str()])?;
                        }
                        _ => {
                            transaction.execute(
                                "UPDATE workspaces SET path = ?2 WHERE id = ?1",
                                params![id.as_str(), canonical],
                            )?;
                            transaction.execute(
                                "UPDATE conversations SET target_path = ?2 WHERE target_workspace_id = ?1",
                                params![id.as_str(), canonical],
                            )?;
                            for member in &inside {
                                super::conversation_index::advance_context(&transaction, member)?;
                            }
                        }
                    }
                    let mut named = DirectoryNamed { conversations: inside, projects: true };
                    // A directory that is a project's worktree is no project
                    // of its own (`crates/backend-database/src/selection.rs` § Worktrees).
                    if let Some(device) = device_in(&transaction, &project.device)?
                        && let Some(super::Selection::Workspace { workspace_id, worktree: Some(worktree), .. }) =
                            super::selection::worktree_at(&transaction, &device, &canonical)?
                    {
                        let folded = fold_into_worktree(&transaction, &device, &workspace_id, &worktree)?;
                        for id in folded.into_iter().flatten() {
                            if !named.conversations.contains(&id) {
                                named.conversations.push(id);
                            }
                        }
                    }
                    named
                }
            };
            transaction.commit()?;
            Ok(named)
        })
        .await
    }
}

/// What a project's reported worktrees changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorktreesStored {
    /// The conversations now in a worktree of the project, which were in a
    /// project of the worktree's own; their pages and agents learn where they
    /// work.
    pub conversations: Vec<ConversationId>,
    /// The projects changed: a worktree's project went into the project.
    pub projects: bool,
}

impl ControlService {
    /// Keeps `worktrees` as the linked worktrees of the project of
    /// `directory` on `device`, as its runner reported them, in place of what
    /// it kept; a directory that is no project's keeps nothing. A worktree
    /// whose directory is a project of its own takes that project's
    /// conversations into this one, with the worktree, and deletes it
    /// (`crates/backend-database/src/selection.rs` § Worktrees).
    pub async fn store_worktrees(
        &self,
        device: DeviceId,
        directory: String,
        worktrees: Vec<super::Worktree>,
    ) -> Result<WorktreesStored, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let Some(device) = device_in(&transaction, &device)? else {
                return Ok(WorktreesStored::default());
            };
            let Some(project) = workspace_at(&transaction, &device.user, &device.id, &directory)? else {
                return Ok(WorktreesStored::default());
            };
            transaction.execute("DELETE FROM workspace_worktrees WHERE workspace_id = ?1", [project.id.as_str()])?;
            let mut stored = WorktreesStored::default();
            for worktree in &worktrees {
                // A repository names each worktree once; a report that named
                // one twice keeps the first.
                transaction.execute(
                    "INSERT INTO workspace_worktrees (workspace_id, name, path) VALUES (?1, ?2, ?3)
                     ON CONFLICT (workspace_id, name) DO NOTHING",
                    params![project.id.as_str(), worktree.name, worktree.path],
                )?;
            }
            for worktree in &worktrees {
                if let Some(folded) = fold_into_worktree(&transaction, &device, &project.id, worktree)? {
                    stored.projects = true;
                    stored.conversations.extend(folded);
                }
            }
            transaction.commit()?;
            Ok(stored)
        })
        .await
    }
}

/// Takes the conversations of the project of `worktree`'s directory on
/// `device`, if one is, into the project `into`, working in `worktree`, and
/// deletes that project; answers the conversations it took, none when no
/// project was deleted.
fn fold_into_worktree(
    connection: &Connection,
    device: &super::devices::DeviceRecord,
    into: &WorkspaceId,
    worktree: &super::Worktree,
) -> Result<Option<Vec<ConversationId>>, StorageError> {
    let Some(own) = workspace_at(connection, &device.user, &device.id, &worktree.path)? else {
        return Ok(None);
    };
    let Some(project) = workspace_by_id(connection, into)? else {
        return Ok(None);
    };
    if own.id == project.id {
        return Ok(None);
    }
    let members: Vec<ConversationId> = connection
        .prepare_cached("SELECT id FROM conversations WHERE target_workspace_id = ?1")?
        .query_map([own.id.as_str()], |row| row.get::<_, String>(0))?
        .map(|id| decode("conversations", "id", ConversationId::try_from(id?.as_str())))
        .collect::<Result<_, _>>()?;
    for member in &members {
        super::devices::set_selection(
            connection,
            member,
            &super::Selection::Workspace {
                workspace_id: project.id.clone(),
                device_id: project.device.clone(),
                path: project.path.clone(),
                worktree: Some(worktree.clone()),
            },
        )?;
        super::conversation_index::advance_context(connection, member)?;
    }
    connection.execute("DELETE FROM workspaces WHERE id = ?1", [own.id.as_str()])?;
    Ok(Some(members))
}

fn workspace_conversations(
    connection: &Connection,
    user: &UserId,
    id: &WorkspaceId,
) -> Result<Vec<ConversationId>, StorageError> {
    connection
        .prepare_cached(
            "SELECT id FROM conversations WHERE user_id = ?1 AND target_workspace_id = ?2 AND archived = 0",
        )?
        .query_map(params![user.as_str(), id.as_str()], |row| row.get::<_, String>(0))?
        .map(|id| decode("conversations", "id", ConversationId::try_from(id?.as_str())))
        .collect()
}

/// The workspace of `id`, in the caller's transaction.
pub(crate) fn workspace_in(
    connection: &Connection,
    id: &WorkspaceId,
) -> Result<Option<WorkspaceRecord>, StorageError> {
    workspace_by_id(connection, id)
}

/// The user's workspace of `path` on `device`, of which there is one at
/// most.
pub(crate) fn workspace_at(
    connection: &Connection,
    user: &UserId,
    device: &DeviceId,
    path: &str,
) -> Result<Option<WorkspaceRecord>, StorageError> {
    let mut statement = connection.prepare_cached(&format!(
        "SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE user_id = ?1 AND device_id = ?2 AND path = ?3"
    ))?;
    let mut rows = statement.query(params![user.as_str(), device.as_str(), path])?;
    rows.next()?.map(workspace_row).transpose()
}

/// Inserts the workspace `id` at `path` on `device`, before the user's
/// others: a new project is listed first.
pub(crate) fn insert_workspace(
    connection: &Connection,
    id: WorkspaceId,
    user: &UserId,
    device: &DeviceId,
    path: &str,
    name: &str,
    now: Timestamp,
) -> Result<WorkspaceRecord, StorageError> {
    connection.execute(
        "INSERT INTO workspaces (id, user_id, device_id, path, name, sort_order, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5,
           (SELECT COALESCE(MIN(sort_order), 1) - 1 FROM workspaces WHERE user_id = ?2), ?6)",
        params![id.as_str(), user.as_str(), device.as_str(), path, name, now.as_millisecond()],
    )?;
    Ok(WorkspaceRecord {
        id,
        user: user.clone(),
        device: device.clone(),
        path: path.to_owned(),
        name: name.to_owned(),
        created_at: now,
        last_model: None,
        last_workflow: None,
    })
}

fn workspace_by_id(
    connection: &Connection,
    id: &WorkspaceId,
) -> Result<Option<WorkspaceRecord>, StorageError> {
    let mut statement = connection.prepare_cached(&format!(
        "SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE id = ?1"
    ))?;
    let mut rows = statement.query([id.as_str()])?;
    rows.next()?.map(workspace_row).transpose()
}

fn workspace_row(row: &Row<'_>) -> Result<WorkspaceRecord, StorageError> {
    const TABLE: &str = "workspaces";
    Ok(WorkspaceRecord {
        id: decode(
            TABLE,
            "id",
            WorkspaceId::try_from(row.get::<_, String>("id")?),
        )?,
        user: decode(
            TABLE,
            "user_id",
            UserId::try_from(row.get::<_, String>("user_id")?),
        )?,
        device: decode(
            TABLE,
            "device_id",
            DeviceId::try_from(row.get::<_, String>("device_id")?),
        )?,
        path: row.get("path")?,
        name: row.get("name")?,
        created_at: instant(row, TABLE, "created_at")?,
        last_model: row
            .get::<_, Option<String>>("last_model")?
            .map(|text| json(TABLE, "last_model", &text))
            .transpose()?,
        last_workflow: row
            .get::<_, Option<String>>("last_workflow_kind")?
            .map(|kind| workflow_choice(TABLE, "last_workflow_kind", &kind, row.get("last_workflow_id")?, false))
            .transpose()?,
    })
}
