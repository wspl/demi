//! Workspaces: the parts of the user's account, each with conversations and
//! projects of its own. Creating, renaming, reordering and moving into one
//! touch no file; deleting one deletes its conversations.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_database::workspaces::{MoveRefusal, WorkspaceChange};
use demi_backend_page_sync::Part;
use demi_backend_user_shard::services::Services;
use demi_backend_user_shard::shard::workspaces::WorkspaceRefusal;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::WorkspaceId;
use demi_web_api_protocol::workspaces::{
    ChangeWorkspace, CreateWorkspace, WorkspaceAnswer, WorkspaceMove, WorkspaceOrder, Workspaces,
};

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;
use super::projects::workspace_not_found;

pub(super) async fn list(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
) -> Result<Json<Workspaces>, ApiError> {
    let workspaces = services.control.workspaces(user.id).await?;
    Ok(Json(Workspaces { workspaces: workspaces.into_iter().map(|workspace| workspace.dto()).collect() }))
}

pub(super) async fn create(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    JsonBody(request): JsonBody<CreateWorkspace>,
) -> Result<(StatusCode, Json<WorkspaceAnswer>), ApiError> {
    let workspace = services.control.create_workspace(user.id.clone(), request.name.into_string()).await?;
    services.sync.mark(&user.id, Part::Workspaces);
    Ok((StatusCode::CREATED, Json(WorkspaceAnswer { workspace: workspace.dto() })))
}

pub(super) async fn change(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(change): JsonBody<ChangeWorkspace>,
) -> Result<Json<WorkspaceAnswer>, ApiError> {
    let id = workspace_id(id)?;
    let change = WorkspaceChange {
        name: change.name.map(|name| name.into_string()),
        icon: change.icon,
        color: change.color,
    };
    let workspace = services
        .control
        .change_workspace(user.id.clone(), id, change)
        .await?
        .ok_or_else(workspace_not_found)?;
    services.sync.mark(&user.id, Part::Workspaces);
    Ok(Json(WorkspaceAnswer { workspace: workspace.dto() }))
}

pub(super) async fn order(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    JsonBody(order): JsonBody<WorkspaceOrder>,
) -> Result<StatusCode, ApiError> {
    if !services.control.order_workspaces(user.id.clone(), order.ids).await? {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::InvalidOrder,
            "The order does not name each of your workspaces once",
        ));
    }
    services.sync.mark(&user.id, Part::Workspaces);
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes the workspace and its conversations, as one step the user's
/// shard takes.
pub(super) async fn delete(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = workspace_id(id)?;
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.delete_workspace(&id).await })
        .await?
        .map_err(|refusal| match refusal {
            WorkspaceRefusal::NotFound => workspace_not_found(),
            WorkspaceRefusal::Last => {
                ApiError::new(StatusCode::CONFLICT, ErrorCode::LastWorkspace, "Your last workspace stays")
            }
            WorkspaceRefusal::Changed => ApiError::new(
                StatusCode::CONFLICT,
                ErrorCode::WorkspaceChanged,
                "A conversation came into the workspace meanwhile; try again",
            ),
            WorkspaceRefusal::Storage(error) => error.into(),
        })?;
    Ok(StatusCode::NO_CONTENT)
}

/// Moves conversations and projects of the user's other workspaces into
/// this one.
pub(super) async fn moves(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(request): JsonBody<WorkspaceMove>,
) -> Result<StatusCode, ApiError> {
    let id = workspace_id(id)?;
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move {
            shard.move_to_workspace(&id, request.conversations, request.projects).await
        })
        .await??
        .map_err(|refusal| match refusal {
            MoveRefusal::WorkspaceNotFound => workspace_not_found(),
            MoveRefusal::ConversationNotFound => {
                ApiError::new(StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound, "No such conversation")
            }
            MoveRefusal::ProjectNotFound => {
                ApiError::new(StatusCode::NOT_FOUND, ErrorCode::ProjectNotFound, "No such project")
            }
        })?;
    Ok(StatusCode::NO_CONTENT)
}

fn workspace_id(id: String) -> Result<WorkspaceId, ApiError> {
    WorkspaceId::try_from(id).map_err(|_| workspace_not_found())
}
