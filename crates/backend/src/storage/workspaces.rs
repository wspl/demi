//! Workspace records (`storage.md` § Control records): a named directory on
//! one of the user's devices, which conversations can target. A workspace is
//! a pointer: it owns no files. A new workspace joins the user's others at
//! the end of their order.

use demi_core::Timestamp;
use demi_web_api::ids::{DeviceId, UserId, WorkspaceId};
use demi_web_api::workspaces::WorkspaceDto;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{decode, instant};
use super::control::ControlService;

const WORKSPACE_COLUMNS: &str = "id, user_id, device_id, path, name, created_at";

/// A `workspaces` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceRecord {
    pub(crate) id: WorkspaceId,
    pub(crate) user: UserId,
    pub(crate) device: DeviceId,
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) created_at: Timestamp,
}

impl WorkspaceRecord {
    /// The workspace as the page lists it.
    pub(crate) fn dto(self) -> WorkspaceDto {
        WorkspaceDto {
            id: self.id,
            device_id: self.device,
            path: self.path,
            name: self.name,
            created_at: self.created_at,
        }
    }
}

/// What a workspace's deletion found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceDeletion {
    Deleted,
    /// The user has no workspace of the id.
    Missing,
    /// That many conversations still target it, so it stays.
    InUse(u64),
}

impl ControlService {
    pub(crate) async fn workspace(&self, id: WorkspaceId) -> Result<Option<WorkspaceRecord>, StorageError> {
        self.call(move |connection, _| workspace_by_id(connection, &id)).await
    }

    /// The user's workspaces, in their order.
    pub(crate) async fn workspaces(&self, user: UserId) -> Result<Vec<WorkspaceRecord>, StorageError> {
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

    /// A new workspace at `path` on the user's `device`, after the user's
    /// others; `None`, writing nothing, when the device is not the user's.
    /// The device is looked up in the same statement, so a revocation cannot
    /// slip between the check and the write.
    pub(crate) async fn create_workspace(
        &self,
        user: UserId,
        device: DeviceId,
        path: String,
        name: String,
    ) -> Result<Option<WorkspaceRecord>, StorageError> {
        let id = WorkspaceId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        self.call(move |connection, now| {
            let created = connection.execute(
                "INSERT INTO workspaces (id, user_id, device_id, path, name, sort_order, created_at)
                 SELECT ?1, ?2, ?3, ?4, ?5,
                   (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM workspaces WHERE user_id = ?2), ?6
                 WHERE EXISTS (SELECT 1 FROM devices WHERE id = ?3 AND user_id = ?2)",
                params![
                    id.as_str(),
                    user.as_str(),
                    device.as_str(),
                    path,
                    name,
                    now.as_millisecond()
                ],
            )?;
            Ok((created > 0).then_some(WorkspaceRecord {
                id,
                user,
                device,
                path,
                name,
                created_at: now,
            }))
        })
        .await
    }

    /// Renames the user's workspace `id` and answers it as it now is; `None`
    /// when the user has no workspace of the id.
    pub(crate) async fn rename_workspace(
        &self,
        user: UserId,
        id: WorkspaceId,
        name: String,
    ) -> Result<Option<WorkspaceRecord>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "UPDATE workspaces SET name = ?1 WHERE id = ?2 AND user_id = ?3 RETURNING {WORKSPACE_COLUMNS}"
            ))?;
            let mut rows = statement.query(params![name, id.as_str(), user.as_str()])?;
            rows.next()?.map(workspace_row).transpose()
        })
        .await
    }

    /// Deletes the user's workspace `id` unless conversations still target
    /// it; the count and the delete are one transaction.
    pub(crate) async fn delete_workspace(&self, user: UserId, id: WorkspaceId) -> Result<WorkspaceDeletion, StorageError> {
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
            let targeting: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM conversations WHERE target_workspace_id = ?1",
                [id.as_str()],
                |row| row.get(0),
            )?;
            let targeting = decode("conversations", "target_workspace_id", u64::try_from(targeting))?;
            if targeting > 0 {
                return Ok(WorkspaceDeletion::InUse(targeting));
            }
            transaction.execute("DELETE FROM workspaces WHERE id = ?1", [id.as_str()])?;
            transaction.commit()?;
            Ok(WorkspaceDeletion::Deleted)
        })
        .await
    }
}

fn workspace_by_id(connection: &Connection, id: &WorkspaceId) -> Result<Option<WorkspaceRecord>, StorageError> {
    let mut statement = connection.prepare_cached(&format!("SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE id = ?1"))?;
    let mut rows = statement.query([id.as_str()])?;
    rows.next()?.map(workspace_row).transpose()
}

fn workspace_row(row: &Row<'_>) -> Result<WorkspaceRecord, StorageError> {
    const TABLE: &str = "workspaces";
    Ok(WorkspaceRecord {
        id: decode(TABLE, "id", WorkspaceId::try_from(row.get::<_, String>("id")?))?,
        user: decode(TABLE, "user_id", UserId::try_from(row.get::<_, String>("user_id")?))?,
        device: decode(TABLE, "device_id", DeviceId::try_from(row.get::<_, String>("device_id")?))?,
        path: row.get("path")?,
        name: row.get("name")?,
        created_at: instant(row, TABLE, "created_at")?,
    })
}
