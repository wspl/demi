//! Workspaces (`web-api.md` § Workspaces, devices, and attached hosts): the
//! user's named directories on their devices, created, renamed and deleted
//! as pointers that never touch a file. A workspace stays while
//! conversations target it.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_database::workspaces::{WorkspaceDeletion, new_workspace_id};
use demi_backend_page_sync::Part;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::WorkspaceId;
use demi_web_api_protocol::workspaces::{CreateWorkspace, RenameWorkspace, WorkspaceAnswer, Workspaces};

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;
use demi_backend_user_shard::services::Services;

pub(super) async fn list(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
) -> Result<Json<Workspaces>, ApiError> {
    let workspaces = services.control.workspaces(user.id).await?;
    Ok(Json(Workspaces {
        workspaces: workspaces.into_iter().map(|workspace| workspace.dto()).collect(),
    }))
}

/// A directory on one of the user's devices, or a new project on the
/// user's Cloud.
pub(super) async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    JsonBody(request): JsonBody<CreateWorkspace>,
) -> Result<(StatusCode, Json<WorkspaceAnswer>), ApiError> {
    let workspace = match request {
        CreateWorkspace::Device { device_id, path, name } => state
            .services
            .control
            .create_workspace(
                new_workspace_id(),
                user.id.clone(),
                device_id,
                path.as_str().to_owned(),
                name.into_string(),
            )
            .await?
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, ErrorCode::DeviceNotFound, "No such device"))?,
        CreateWorkspace::Cloud { name } => {
            let name = name.into_string();
            state
                .shards
                .of(&user.id)
                .call(move |shard, _| async move { shard.create_cloud_workspace(name).await })
                .await??
        }
    };
    state.services.sync.mark(&user.id, Part::Workspaces);
    Ok((
        StatusCode::CREATED,
        Json(WorkspaceAnswer {
            workspace: workspace.dto(),
        }),
    ))
}

pub(super) async fn rename(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(RenameWorkspace { name }): JsonBody<RenameWorkspace>,
) -> Result<Json<WorkspaceAnswer>, ApiError> {
    let id = workspace_id(id)?;
    let workspace = services
        .control
        .rename_workspace(user.id.clone(), id, name.into_string())
        .await?
        .ok_or_else(not_found)?;
    services.sync.mark(&user.id, Part::Workspaces);
    Ok(Json(WorkspaceAnswer {
        workspace: workspace.dto(),
    }))
}

pub(super) async fn delete(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = workspace_id(id)?;
    match services.control.delete_workspace(user.id.clone(), id).await? {
        WorkspaceDeletion::Deleted => {
            services.sync.mark(&user.id, Part::Workspaces);
            Ok(StatusCode::NO_CONTENT)
        }
        WorkspaceDeletion::Missing => Err(not_found()),
        WorkspaceDeletion::InUse(conversations) => Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::WorkspaceInUse,
            format!("{conversations} conversation(s) still target this workspace"),
        )),
    }
}

fn workspace_id(id: String) -> Result<WorkspaceId, ApiError> {
    WorkspaceId::try_from(id).map_err(|_| not_found())
}

fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, ErrorCode::WorkspaceNotFound, "No such workspace")
}
