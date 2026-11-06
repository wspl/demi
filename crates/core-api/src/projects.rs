//! Projects: the user's directories on their devices, each named by its
//! directory.
//!
//! They are created and deleted as pointers that never touch a file, but a
//! new Cloud project's directory, which its creation makes. Deleting one
//! archives its conversations and keeps it, marked deleted, to come back.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_core_store::saved::ScopeSwitch;
use demi_core_store::projects::{ProjectChange, ProjectCreation, new_project_id};
use demi_core_sync::Part;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::ProjectId;
use demi_web_api_protocol::projects::{
    ChangeProject, CreateProject, SavedScopeSwitch, ProjectAnswer, Projects,
};

use crate::api::Api;
use crate::body::JsonBody;
use crate::error::ApiError;
use crate::caller::Principal;
use demi_core::conversation::CloudProject;
use demi_core::services::CoreServices;

pub(crate) async fn list(
    State(services): State<Arc<CoreServices>>,
    Principal(user): Principal,
) -> Result<Json<Projects>, ApiError> {
    let projects = services.vault.stores().store_of(&user).await?.projects().await?;
    Ok(Json(Projects {
        projects: projects
            .into_iter()
            .map(|project| project.dto())
            .collect(),
    }))
}

/// A directory on one of the user's devices, or a new project on the
/// user's Cloud.
pub(crate) async fn create(
    State(state): State<Api>,
    Principal(user): Principal,
    JsonBody(request): JsonBody<CreateProject>,
) -> Result<(StatusCode, Json<ProjectAnswer>), ApiError> {
    let project = match request {
        CreateProject::Device { workspace_id, device_id, path } => {
            // The directory as its Host names it, so two spellings of one
            // directory are one project (`crates/core-store/src/selection.rs`).
            let resolving = device_id.clone();
            let path = state
                .shards
                .of(&user)
                .call(move |core, _| async move {
                    core.host_core().device_directory(&resolving, path.as_str()).await
                })
                .await?
                .map_err(crate::hosts::refused)?;
            match state
            .services.vault.stores().store_of(&user).await?.create_project(new_project_id(), workspace_id, device_id, path)
            .await?
        {
            ProjectCreation::Created(project) => project,
            ProjectCreation::Exists(project) => {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    ErrorCode::ProjectExists,
                    format!("The project {} has this directory", project.name),
                )
                .with_project(project.dto()));
            }
            ProjectCreation::Home => {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    ErrorCode::ProjectIsHome,
                    "Your home folder holds the conversations outside projects. Choose a folder in it.",
                ));
            }
            ProjectCreation::DeviceNotFound => {
                return Err(ApiError::new(
                    StatusCode::NOT_FOUND,
                    ErrorCode::DeviceNotFound,
                    "No such device",
                ));
            }
            ProjectCreation::WorkspaceNotFound => return Err(workspace_not_found()),
        }
        }
        CreateProject::Cloud { workspace_id, name } => {
            let name = name.into_string();
            let shown = name.clone();
            match state
                .shards
                .of(&user)
                .call(move |core, _| async move { core.create_cloud_project(workspace_id, name).await })
                .await??
            {
                CloudProject::Made(project) => project,
                CloudProject::NameTaken => {
                    return Err(ApiError::new(
                        StatusCode::CONFLICT,
                        ErrorCode::ProjectNameTaken,
                        format!("The name “{shown}” is already taken. Choose a different name."),
                    )
                    .with_param("name", shown));
                }
                CloudProject::NameInvalid => {
                    return Err(ApiError::new(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        ErrorCode::ProjectNameInvalid,
                        "A project’s name can’t contain “/” or “\\”, or be “.” or “..”.",
                    ));
                }
                CloudProject::WorkspaceNotFound => return Err(workspace_not_found()),
            }
        }
    };
    // Through the core, which also tells the Host's runner the new
    // project's directory, whose worktrees it reports.
    state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.mark(Part::Projects) })
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(ProjectAnswer {
            project: project.dto(),
        }),
    ))
}

/// What the project's next conversation starts with.
pub(crate) async fn change(
    State(services): State<Arc<CoreServices>>,
    Principal(user): Principal,
    Path(id): Path<String>,
    JsonBody(change): JsonBody<ChangeProject>,
) -> Result<Json<ProjectAnswer>, ApiError> {
    let id = project_id(id)?;
    let change = ProjectChange {
        last_model: change.last_model,
        last_workflow: change.last_workflow,
    };
    let project = services.vault.stores().store_of(&user).await?.change_project(id, change)
        .await?
        .ok_or_else(not_found)?;
    services.sync.mark(&user, Part::Projects);
    Ok(Json(ProjectAnswer {
        project: project.dto(),
    }))
}

/// Switches how the project's conversations keep their saved messages,
/// moving them in the same step; every page that shows one of the
/// conversations whose list changed receives its new list.
pub(crate) async fn switch_saved_scope(
    State(services): State<Arc<CoreServices>>,
    Principal(user): Principal,
    Path(id): Path<String>,
    JsonBody(switch): JsonBody<SavedScopeSwitch>,
) -> Result<Json<ProjectAnswer>, ApiError> {
    let id = project_id(id)?;
    let switched = services.vault.stores().store_of(&user).await?.switch_saved_scope(id.clone(), switch.scope, switch.to)
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
        services.sync.mark(&user, Part::Metadata(conversation));
    }
    services.sync.mark(&user, Part::Projects);
    let project = services.vault.stores().store_of(&user).await?.project(id).await?.ok_or_else(not_found)?;
    Ok(Json(ProjectAnswer {
        project: project.dto(),
    }))
}

/// Archives the project's conversations and deletes it, as one transition
/// the user's core makes.
pub(crate) async fn delete(
    State(state): State<Api>,
    Principal(user): Principal,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = project_id(id)?;
    state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.delete_project(&id).await })
        .await?
        .map_err(crate::hosts::refused)?;
    Ok(StatusCode::NO_CONTENT)
}

fn project_id(id: String) -> Result<ProjectId, ApiError> {
    ProjectId::try_from(id).map_err(|_| not_found())
}

fn not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        ErrorCode::ProjectNotFound,
        "No such project",
    )
}

/// The refusal of a workspace the user does not have.
pub(crate) fn workspace_not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, ErrorCode::WorkspaceNotFound, "No such workspace")
}
