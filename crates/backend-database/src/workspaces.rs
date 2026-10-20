//! Workspace records: a named directory on one of the user's devices, which
//! conversations can target. A workspace is a pointer: it owns no files, and
//! the user has one per device and directory. A new workspace joins the
//! user's others at the end of their order. It keeps the model settings and
//! the workflow the project's next conversation starts with.

use demi_shared_types::{Timestamp, WorkflowChoice};
use demi_web_api_protocol::conversations::ModelSettings;
use demi_backend_remote_host::host_paths;
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

    /// The new workspace `id` at `path` on the user's `device`, after the
    /// user's others, unless the device is not the user's, the directory is
    /// its home, or the user has a workspace of it already, which it
    /// answers. The checks and the write are one transaction, so a
    /// revocation cannot slip between them.
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
                Some(choice) => {
                    let (kind, workflow) = workflow_columns(choice);
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

/// Inserts the workspace `id` at `path` on `device`, after the user's
/// others.
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
           (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM workspaces WHERE user_id = ?2), ?6)",
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
            .map(|kind| workflow_choice(TABLE, "last_workflow_kind", &kind, row.get("last_workflow_id")?))
            .transpose()?,
    })
}
