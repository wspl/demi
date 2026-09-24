//! Workspace records (`storage.md` § Control records): a named directory on
//! one of the user's devices, which conversations can target. A workspace is
//! a pointer: it owns no files.

use demi_web_api::ids::{DeviceId, UserId, WorkspaceId};
use rusqlite::{OptionalExtension, Row};

use super::StorageError;
use super::columns::decode;
use super::control::ControlService;

/// A `workspaces` row, as far as a conversation's target needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceRecord {
    pub(crate) id: WorkspaceId,
    pub(crate) user: UserId,
    pub(crate) device: DeviceId,
    pub(crate) path: String,
    pub(crate) name: String,
}

impl ControlService {
    pub(crate) async fn workspace(&self, id: WorkspaceId) -> Result<Option<WorkspaceRecord>, StorageError> {
        self.call(move |connection, _| {
            connection
                .query_row(
                    "SELECT id, user_id, device_id, path, name FROM workspaces WHERE id = ?1",
                    [id.as_str()],
                    |row| Ok(workspace_row(row)),
                )
                .optional()?
                .transpose()
        })
        .await
    }
}

fn workspace_row(row: &Row<'_>) -> Result<WorkspaceRecord, StorageError> {
    const TABLE: &str = "workspaces";
    Ok(WorkspaceRecord {
        id: decode(TABLE, "id", WorkspaceId::try_from(row.get::<_, String>("id")?))?,
        user: decode(TABLE, "user_id", UserId::try_from(row.get::<_, String>("user_id")?))?,
        device: decode(TABLE, "device_id", DeviceId::try_from(row.get::<_, String>("device_id")?))?,
        path: row.get("path")?,
        name: row.get("name")?,
    })
}
