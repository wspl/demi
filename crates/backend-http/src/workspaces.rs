//! Workspaces (`web-api-hosts.md` § Workspaces, devices, and attached hosts): the
//! user's named directories on their devices, created, renamed and deleted
//! as pointers that never touch a file. Deleting one archives its
//! conversations (`conversations.md` § Conversations and projects).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_database::workspaces::{WorkspaceChange, WorkspaceCreation, new_workspace_id};
use demi_backend_page_sync::Part;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::WorkspaceId;
use demi_web_api_protocol::text::Trimmed;
use demi_web_api_protocol::workspaces::{
    ChangeWorkspace, CreateWorkspace, WorkspaceAnswer, Workspaces,
};

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
        workspaces: workspaces
            .into_iter()
            .map(|workspace| workspace.dto())
            .collect(),
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
        CreateWorkspace::Device {
            device_id,
            path,
            name,
        } => {
            // The directory as its Host names it, so two spellings of one
            // directory are one project (`sessions-and-targets.md` § Resolve
            // a target).
            let resolving = device_id.clone();
            let path = state
                .shards
                .of(&user.id)
                .call(move |shard, _| async move {
                    shard.host_shard().device_directory(&resolving, path.as_str()).await
                })
                .await?
                .map_err(super::hosts::refused)?;
            match state
            .services
            .control
            .create_workspace(new_workspace_id(), user.id.clone(), device_id, path, name.into_string())
            .await?
        {
            WorkspaceCreation::Created(workspace) => workspace,
            WorkspaceCreation::Exists(workspace) => {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    ErrorCode::WorkspaceExists,
                    format!("The project {} has this directory", workspace.name),
                )
                .with_workspace(workspace.dto()));
            }
            WorkspaceCreation::Home => {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    ErrorCode::WorkspaceIsHome,
                    "Your home folder holds the conversations outside projects. Choose a folder in it.",
                ));
            }
            WorkspaceCreation::DeviceNotFound => {
                return Err(ApiError::new(
                    StatusCode::NOT_FOUND,
                    ErrorCode::DeviceNotFound,
                    "No such device",
                ));
            }
        }
        }
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

/// A new name, or what the project's next conversation starts with.
pub(super) async fn change(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(change): JsonBody<ChangeWorkspace>,
) -> Result<Json<WorkspaceAnswer>, ApiError> {
    let id = workspace_id(id)?;
    let change = WorkspaceChange {
        name: change.name.map(Trimmed::into_string),
        last_model: change.last_model,
        last_workflow: change.last_workflow,
    };
    let workspace = services
        .control
        .change_workspace(user.id.clone(), id, change)
        .await?
        .ok_or_else(not_found)?;
    services.sync.mark(&user.id, Part::Workspaces);
    Ok(Json(WorkspaceAnswer {
        workspace: workspace.dto(),
    }))
}

/// Archives the project's conversations and deletes it, as one transition
/// the user's shard makes.
pub(super) async fn delete(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = workspace_id(id)?;
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.delete_project(&id).await })
        .await?
        .map_err(super::hosts::refused)?;
    Ok(StatusCode::NO_CONTENT)
}

fn workspace_id(id: String) -> Result<WorkspaceId, ApiError> {
    WorkspaceId::try_from(id).map_err(|_| not_found())
}

fn not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        ErrorCode::WorkspaceNotFound,
        "No such workspace",
    )
}
