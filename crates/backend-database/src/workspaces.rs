//! Workspace records: the parts of a user's account, each with conversations
//! and projects of its own.
//!
//! - A user has at least one: an account starts with one, whose name is
//!   null until the user names it, and the last one cannot be deleted.
//! - A conversation is in one workspace, and in a project only of its own
//!   workspace; the conversations' key of their project and workspace keeps
//!   them together, and a project's move takes its conversations along.
//! - One directory may be a project in several workspaces. A conversation
//!   that moves to another workspace joins that workspace's project of its
//!   directory, made when there is none; a project that moves merges into
//!   the other workspace's project of its directory, or moves whole.

use demi_shared_types::Timestamp;
use demi_web_api_protocol::ids::{ConversationId, ProjectId, UserId, WorkspaceId};
use demi_web_api_protocol::workspaces::{WorkspaceColor, WorkspaceDto, WorkspaceIcon};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::columns::decode;
use super::control::ControlService;
use super::selection::Selection;

const WORKSPACE_COLUMNS: &str = "id, user_id, name, icon, color";

/// A new workspace's id, which the backend assigns.
pub fn new_workspace_id() -> WorkspaceId {
    WorkspaceId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty")
}

/// The name of the workspace an account starts with while the user has not
/// named it, as Demi's commands say it; each page says it in its language.
pub const FIRST_WORKSPACE_NAME: &str = "Personal";

/// A `workspaces` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRecord {
    pub id: WorkspaceId,
    pub user: UserId,
    /// Its name; none while it keeps the one an account starts with.
    pub name: Option<String>,
    pub icon: WorkspaceIcon,
    pub color: WorkspaceColor,
}

impl WorkspaceRecord {
    /// Its name as Demi's commands say it.
    pub fn shown_name(&self) -> &str {
        self.name.as_deref().unwrap_or(FIRST_WORKSPACE_NAME)
    }

    /// The workspace as the page lists it.
    pub fn dto(self) -> WorkspaceDto {
        WorkspaceDto { id: self.id, name: self.name, icon: self.icon, color: self.color }
    }
}

/// A change of a workspace: each field it holds replaces the stored one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceChange {
    pub name: Option<String>,
    pub icon: Option<WorkspaceIcon>,
    pub color: Option<WorkspaceColor>,
}

/// What a workspace's deletion did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceDeletion {
    Deleted,
    /// The user has no workspace of the id.
    Missing,
    /// It is the user's last workspace, which stays.
    Last,
    /// A conversation is still in it: one moved in or was created after its
    /// conversations were deleted. Nothing changed.
    Changed,
}

/// What a move into a workspace changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceMoved {
    /// The conversations now in the workspace, whose summaries change.
    pub conversations: Vec<ConversationId>,
    /// Whether a project was made, moved, merged or deleted.
    pub projects: bool,
    /// The conversations whose list of saved messages changed, whose pages
    /// read it again.
    pub saved: Vec<ConversationId>,
}

/// Why a move into a workspace was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveRefusal {
    /// The user has no workspace of the id.
    WorkspaceNotFound,
    /// The user has no conversation of one of the ids.
    ConversationNotFound,
    /// The user has no project of one of the ids.
    ProjectNotFound,
}

impl ControlService {
    /// The user's workspaces, in their order.
    pub async fn workspaces(&self, user: UserId) -> Result<Vec<WorkspaceRecord>, StorageError> {
        self.call(move |connection, _| workspaces_of(connection, &user)).await
    }

    /// The workspace `id`, the user's or another's.
    pub async fn workspace(&self, id: WorkspaceId) -> Result<Option<WorkspaceRecord>, StorageError> {
        self.call(move |connection, _| workspace_by_id(connection, &id)).await
    }

    /// The user's new workspace `name`, last in their order, with the icon and
    /// the color after the ones the user's last workspace has.
    pub async fn create_workspace(&self, user: UserId, name: String) -> Result<WorkspaceRecord, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let last = workspaces_of(&transaction, &user)?.pop();
            let next = |position: Option<usize>, count: usize| position.map_or(0, |at| (at + 1) % count);
            let icon = WorkspaceIcon::ALL[next(
                last.as_ref().and_then(|last| WorkspaceIcon::ALL.iter().position(|icon| *icon == last.icon)),
                WorkspaceIcon::ALL.len(),
            )];
            let color = WorkspaceColor::ALL[next(
                last.as_ref().and_then(|last| WorkspaceColor::ALL.iter().position(|color| *color == last.color)),
                WorkspaceColor::ALL.len(),
            )];
            let id = insert_workspace(&transaction, &user, Some(&name), icon, color, now)?;
            let record = workspace_by_id(&transaction, &id)?.expect("the workspace was just inserted");
            transaction.commit()?;
            Ok(record)
        })
        .await
    }

    /// Applies `change` to the user's workspace `id` and answers it as it now
    /// is; none when the user has no workspace of the id.
    pub async fn change_workspace(
        &self,
        user: UserId,
        id: WorkspaceId,
        change: WorkspaceChange,
    ) -> Result<Option<WorkspaceRecord>, StorageError> {
        self.call(move |connection, _| {
            let changed = connection.execute(
                "UPDATE workspaces SET
                   name = COALESCE(?1, name),
                   icon = COALESCE(?2, icon),
                   color = COALESCE(?3, color)
                 WHERE id = ?4 AND user_id = ?5",
                params![
                    change.name,
                    change.icon.map(icon_name),
                    change.color.map(color_name),
                    id.as_str(),
                    user.as_str()
                ],
            )?;
            if changed == 0 {
                return Ok(None);
            }
            workspace_by_id(connection, &id)
        })
        .await
    }

    /// Puts the user's workspaces in the order of `ids`; false, changing
    /// nothing, when `ids` are not each of the user's workspaces once.
    pub async fn order_workspaces(&self, user: UserId, ids: Vec<WorkspaceId>) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let mut held: Vec<WorkspaceId> = workspaces_of(&transaction, &user)?.into_iter().map(|record| record.id).collect();
            let mut asked = ids.clone();
            held.sort();
            asked.sort();
            if held != asked {
                return Ok(false);
            }
            for (position, id) in ids.iter().enumerate() {
                transaction.execute(
                    "UPDATE workspaces SET sort_order = ?2 WHERE id = ?1",
                    params![id.as_str(), i64::try_from(position).unwrap_or(i64::MAX)],
                )?;
            }
            transaction.commit()?;
            Ok(true)
        })
        .await
    }

    /// Every conversation in the user's workspace `id`, archived ones
    /// included; none when the user has no workspace of the id.
    pub async fn workspace_conversations(
        &self,
        user: UserId,
        id: WorkspaceId,
    ) -> Result<Option<Vec<ConversationId>>, StorageError> {
        self.call(move |connection, _| {
            if !owns(connection, &user, &id)? {
                return Ok(None);
            }
            Ok(Some(conversations_in(connection, &id)?))
        })
        .await
    }

    /// Deletes the user's workspace `id`, its projects and its plugins' Host
    /// directories, once its conversations are deleted, as one transaction.
    pub async fn delete_workspace(&self, user: UserId, id: WorkspaceId) -> Result<WorkspaceDeletion, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            if !owns(&transaction, &user, &id)? {
                return Ok(WorkspaceDeletion::Missing);
            }
            if workspaces_of(&transaction, &user)?.len() == 1 {
                return Ok(WorkspaceDeletion::Last);
            }
            if !conversations_in(&transaction, &id)?.is_empty() {
                return Ok(WorkspaceDeletion::Changed);
            }
            transaction.execute("DELETE FROM projects WHERE workspace_id = ?1", [id.as_str()])?;
            transaction.execute("DELETE FROM plugin_directories WHERE workspace_id = ?1", [id.as_str()])?;
            transaction.execute("DELETE FROM workspaces WHERE id = ?1", [id.as_str()])?;
            transaction.commit()?;
            Ok(WorkspaceDeletion::Deleted)
        })
        .await
    }

    /// Moves the user's `conversations` and `projects` into the user's
    /// workspace `id`, as one transaction; one in it already stays.
    ///
    /// - A conversation in a project joins the workspace's project of its
    ///   directory, made or brought back when the workspace has none, with
    ///   the worktree it works in.
    /// - A project merges into the workspace's project of its directory, which
    ///   keeps its own choices and takes its conversations, archived ones
    ///   included, and its shared saved messages; with none, it moves whole.
    pub async fn move_to_workspace(
        &self,
        user: UserId,
        id: WorkspaceId,
        conversations: Vec<ConversationId>,
        projects: Vec<ProjectId>,
    ) -> Result<Result<WorkspaceMoved, MoveRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            if !owns(&transaction, &user, &id)? {
                return Ok(Err(MoveRefusal::WorkspaceNotFound));
            }
            let mut moved = WorkspaceMoved::default();
            for project in &projects {
                let Some(record) = super::projects::project_in(&transaction, project)?
                    .filter(|record| record.user == user && !record.deleted)
                else {
                    return Ok(Err(MoveRefusal::ProjectNotFound));
                };
                if record.workspace == id {
                    continue;
                }
                let members = members_of(&transaction, project)?;
                match super::projects::project_at(&transaction, &id, &record.device, &record.path)? {
                    Some(into) => {
                        super::projects::revive(&transaction, &into.id)?;
                        moved.saved.extend(super::saved::project_ends(&transaction, project, Some(&into.id), now)?);
                        transaction.execute(
                            "UPDATE conversations SET target_project_id = ?2, workspace_id = ?3
                             WHERE target_project_id = ?1",
                            params![project.as_str(), into.id.as_str(), id.as_str()],
                        )?;
                        transaction.execute("DELETE FROM projects WHERE id = ?1", [project.as_str()])?;
                    }
                    // The conversations' key of their project takes them along.
                    None => {
                        transaction.execute(
                            "UPDATE projects SET workspace_id = ?2 WHERE id = ?1",
                            params![project.as_str(), id.as_str()],
                        )?;
                    }
                }
                moved.projects = true;
                moved.conversations.extend(members);
            }
            for conversation in &conversations {
                let Some(record) = super::conversation_index::conversation_by_id(&transaction, conversation)?
                    .filter(|record| record.owner == user)
                else {
                    return Ok(Err(MoveRefusal::ConversationNotFound));
                };
                if record.workspace == id {
                    continue;
                }
                match &record.target {
                    Selection::Project { project_id, device_id, path, .. } => {
                        let into = match super::projects::project_at(&transaction, &id, device_id, path)? {
                            Some(into) => {
                                super::projects::revive(&transaction, &into.id)?;
                                into.id
                            }
                            None => {
                                super::projects::insert_project(
                                    &transaction,
                                    super::projects::new_project_id(),
                                    &user,
                                    &id,
                                    device_id,
                                    path,
                                    now,
                                )?
                                .id
                            }
                        };
                        transaction.execute(
                            "UPDATE conversations SET target_project_id = ?2, workspace_id = ?3 WHERE id = ?1",
                            params![conversation.as_str(), into.as_str(), id.as_str()],
                        )?;
                        super::saved::moved(&transaction, conversation, Some(project_id), Some(&into), now)?;
                        moved.saved.push(conversation.clone());
                        moved.projects = true;
                    }
                    _ => {
                        transaction.execute(
                            "UPDATE conversations SET workspace_id = ?2 WHERE id = ?1",
                            params![conversation.as_str(), id.as_str()],
                        )?;
                    }
                }
                moved.conversations.push(conversation.clone());
            }
            transaction.commit()?;
            moved.conversations.sort();
            moved.conversations.dedup();
            moved.saved.sort();
            moved.saved.dedup();
            Ok(Ok(moved))
        })
        .await
    }
}

/// Whether the workspace `id` is `user`'s.
pub(crate) fn owns(connection: &Connection, user: &UserId, id: &WorkspaceId) -> Result<bool, StorageError> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM workspaces WHERE id = ?1 AND user_id = ?2",
            params![id.as_str(), user.as_str()],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// The user's first workspace in their order, which every user has.
pub(crate) fn first(connection: &Connection, user: &UserId) -> Result<WorkspaceId, StorageError> {
    workspaces_of(connection, user)?
        .into_iter()
        .next()
        .map(|record| record.id)
        .ok_or_else(|| StorageError::Corrupt {
            table: "workspaces",
            column: "user_id",
            reason: format!("the user {user} has no workspace"),
        })
}

/// The workspace a new account starts with, under the name it starts with.
pub(crate) fn insert_first(connection: &Connection, user: &UserId, now: Timestamp) -> Result<WorkspaceId, StorageError> {
    insert_workspace(connection, user, None, WorkspaceIcon::User, WorkspaceColor::Blue, now)
}

fn insert_workspace(
    connection: &Connection,
    user: &UserId,
    name: Option<&str>,
    icon: WorkspaceIcon,
    color: WorkspaceColor,
    now: Timestamp,
) -> Result<WorkspaceId, StorageError> {
    let id = new_workspace_id();
    connection.execute(
        "INSERT INTO workspaces (id, user_id, name, icon, color, sort_order, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5,
           (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM workspaces WHERE user_id = ?2), ?6)",
        params![id.as_str(), user.as_str(), name, icon_name(icon), color_name(color), now.as_millisecond()],
    )?;
    Ok(id)
}

fn workspaces_of(connection: &Connection, user: &UserId) -> Result<Vec<WorkspaceRecord>, StorageError> {
    let mut statement = connection
        .prepare_cached(&format!("SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE user_id = ?1 ORDER BY sort_order, id"))?;
    let mut rows = statement.query([user.as_str()])?;
    let mut workspaces = Vec::new();
    while let Some(row) = rows.next()? {
        workspaces.push(workspace_row(row)?);
    }
    Ok(workspaces)
}

fn workspace_by_id(connection: &Connection, id: &WorkspaceId) -> Result<Option<WorkspaceRecord>, StorageError> {
    let mut statement = connection.prepare_cached(&format!("SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE id = ?1"))?;
    let mut rows = statement.query([id.as_str()])?;
    rows.next()?.map(workspace_row).transpose()
}

fn conversations_in(connection: &Connection, id: &WorkspaceId) -> Result<Vec<ConversationId>, StorageError> {
    connection
        .prepare_cached("SELECT id FROM conversations WHERE workspace_id = ?1")?
        .query_map([id.as_str()], |row| row.get::<_, String>(0))?
        .map(|id| decode("conversations", "id", ConversationId::try_from(id?.as_str())))
        .collect()
}

fn members_of(connection: &Connection, project: &ProjectId) -> Result<Vec<ConversationId>, StorageError> {
    connection
        .prepare_cached("SELECT id FROM conversations WHERE target_project_id = ?1")?
        .query_map([project.as_str()], |row| row.get::<_, String>(0))?
        .map(|id| decode("conversations", "id", ConversationId::try_from(id?.as_str())))
        .collect()
}

fn workspace_row(row: &Row<'_>) -> Result<WorkspaceRecord, StorageError> {
    const TABLE: &str = "workspaces";
    let icon: String = row.get("icon")?;
    let color: String = row.get("color")?;
    Ok(WorkspaceRecord {
        id: decode(TABLE, "id", WorkspaceId::try_from(row.get::<_, String>("id")?))?,
        user: decode(TABLE, "user_id", UserId::try_from(row.get::<_, String>("user_id")?))?,
        name: row.get("name")?,
        icon: WorkspaceIcon::ALL.into_iter().find(|candidate| icon_name(*candidate) == icon).ok_or_else(|| {
            StorageError::Corrupt { table: TABLE, column: "icon", reason: format!("no icon is named {icon}") }
        })?,
        color: WorkspaceColor::ALL.into_iter().find(|candidate| color_name(*candidate) == color).ok_or_else(|| {
            StorageError::Corrupt { table: TABLE, column: "color", reason: format!("no color is named {color}") }
        })?,
    })
}

/// An icon's name as the wire and the table spell it.
fn icon_name(icon: WorkspaceIcon) -> String {
    serde_json::to_value(icon).ok().and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_default()
}

/// A color's name as the wire and the table spell it.
fn color_name(color: WorkspaceColor) -> String {
    serde_json::to_value(color).ok().and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_default()
}
