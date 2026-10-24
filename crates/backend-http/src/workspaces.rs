//! Workspaces: the user's directories on their devices, each named by its
//! directory.
//!
//! They are created and deleted as pointers that never touch a file, but a
//! new Cloud project's directory, which its creation makes. Deleting one
//! archives its conversations and keeps it, marked deleted, to come back.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_database::saved::ScopeSwitch;
use demi_backend_database::workspaces::{WorkspaceChange, WorkspaceCreation, new_workspace_id};
use demi_backend_page_sync::Part;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::WorkspaceId;
use demi_web_api_protocol::workspaces::{
    ChangeWorkspace, CreateWorkspace, SavedScopeSwitch, WorkspaceAnswer, Workspaces,
};

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;
use demi_backend_user_shard::conversation::CloudProject;
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
        CreateWorkspace::Device { device_id, path } => {
            // The directory as its Host names it, so two spellings of one
            // directory are one project (`crates/backend-database/src/selection.rs`).
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
            .create_workspace(new_workspace_id(), user.id.clone(), device_id, path)
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
            let shown = name.clone();
            match state
                .shards
                .of(&user.id)
                .call(move |shard, _| async move { shard.create_cloud_workspace(name).await })
                .await??
            {
                CloudProject::Made(workspace) => workspace,
                CloudProject::NameTaken => {
                    return Err(ApiError::new(
                        StatusCode::CONFLICT,
                        ErrorCode::WorkspaceNameTaken,
                        format!("The name “{shown}” is already taken. Choose a different name."),
                    )
                    .with_param("name", shown));
                }
                CloudProject::NameInvalid => {
                    return Err(ApiError::new(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        ErrorCode::WorkspaceNameInvalid,
                        "A project’s name can’t contain “/” or “\\”, or be “.” or “..”.",
                    ));
                }
            }
        }
    };
    // Through the shard, which also tells the Host's runner the new
    // project's directory, whose worktrees it reports.
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.mark(Part::Workspaces) })
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(WorkspaceAnswer {
            workspace: workspace.dto(),
        }),
    ))
}

/// What the project's next conversation starts with.
pub(super) async fn change(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(change): JsonBody<ChangeWorkspace>,
) -> Result<Json<WorkspaceAnswer>, ApiError> {
    let id = workspace_id(id)?;
    let change = WorkspaceChange {
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

/// Switches how the project's conversations keep their saved messages,
/// moving them in the same step; every page that shows one of the
/// conversations whose list changed receives its new list.
pub(super) async fn switch_saved_scope(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(switch): JsonBody<SavedScopeSwitch>,
) -> Result<Json<WorkspaceAnswer>, ApiError> {
    let id = workspace_id(id)?;
    let switched = services
        .control
        .switch_saved_scope(user.id.clone(), id.clone(), switch.scope, switch.to)
        .await?;
    let changed = match switched {
        ScopeSwitch::Switched(changed) => changed,
        ScopeSwitch::Missing => return Err(not_found()),
        ScopeSwitch::DestinationNeeded => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                ErrorCode::SavedDestinationNeeded,
                "The project has saved messages: name the conversation that takes them",
            ));
        }
        ScopeSwitch::DestinationInvalid => {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                ErrorCode::SavedDestinationInvalid,
                "That conversation is not one of the project's that is not archived",
            ));
        }
    };
    for conversation in changed {
        services.sync.mark(&user.id, Part::Metadata(conversation));
    }
    services.sync.mark(&user.id, Part::Workspaces);
    let workspace = services.control.workspace(id).await?.ok_or_else(not_found)?;
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
