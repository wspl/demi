//! `/api/devices` (`web-api.md` § Workspaces, devices, and attached hosts,
//! § Device files and remote references, § Device log): the caller's
//! devices, pairing, renaming and routes, revocation, browsing a paired
//! device's directories to choose a target, and a Host's log. Browsing and the log
//! reach the device through device access, which wakes nothing: a device
//! whose runner is not connected answers 409 `device_offline`.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_database::devices::{DeviceChange, DeviceRecord};
use demi_web_api_protocol::text::Trimmed;
use demi_backend_runners::claims::ClaimRefusal;
use demi_backend_runners::files::browse_directory;
use demi_host_interface::{HostError, HostErrorKind, MkdirOptions};
use demi_web_api_protocol::devices::{
    ChangeDevice, Claim, DeviceAnswer, DeviceKind, DeviceLog, DeviceLogLine, DeviceLogQuery,
    Devices, RevokedDevice,
};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::files::{
    CreateDeviceDirectory, CreatedDirectory, DeviceDirectoryQuery, Directory,
};
use demi_web_api_protocol::ids::DeviceId;
use demi_web_api_protocol::workflows::WorkflowsQuery;
use demi_backend_user_shard::conversation::workflows::DeviceWorkflowsRefusal;

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;
use super::query::QueryParams;

pub(super) async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<Devices>, ApiError> {
    let devices = state
        .shards
        .of(&user.id)
        .call(|shard, _| async move { shard.device_list().await })
        .await??;
    let devices = devices
        .into_iter()
        .filter(|device| device.kind == DeviceKind::User)
        .collect();
    Ok(Json(Devices { devices }))
}

/// Pairs the runner waiting under the code with the caller: the device is
/// made, its token goes to that runner alone, and the runner's socket moves
/// into the caller's shard.
pub(super) async fn claim(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    JsonBody(Claim { code }): JsonBody<Claim>,
) -> Result<(StatusCode, Json<DeviceAnswer>), ApiError> {
    let services = &state.services;
    match services.claims.claim(&services.control, &user.id, &code).await {
        Ok(device) => Ok((StatusCode::CREATED, Json(DeviceAnswer { device }))),
        Err(ClaimRefusal::TooManyAttempts) => Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::RateLimited,
            "Too many claim attempts",
        )),
        Err(ClaimRefusal::UnknownCode) => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::InvalidCode,
            "Unknown or expired pairing code",
        )),
        Err(ClaimRefusal::Storage(error)) => Err(error.into()),
    }
}

/// Renames a paired device, or sets its route; the Cloud keeps its name and
/// is always reached through the server.
pub(super) async fn change(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(ChangeDevice { name, route }): JsonBody<ChangeDevice>,
) -> Result<Json<DeviceAnswer>, ApiError> {
    let device = owned_device(&state, &user.id, &id, None).await?;
    if device.kind == DeviceKind::Managed {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::DeviceManaged,
            "The Cloud keeps its name and is always reached through the server",
        ));
    }
    let change = DeviceChange {
        name: name.map(Trimmed::into_string),
        route,
    };
    let changed = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.change_device(device.id, change).await })
        .await??;
    // A revocation between the lookup and the change took the device.
    let device = changed.ok_or_else(device_not_found)?;
    Ok(Json(DeviceAnswer { device }))
}

/// Revokes a paired device, which nothing refuses; its workspaces go with
/// it.
pub(super) async fn revoke(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<RevokedDevice>, ApiError> {
    let device = owned_device(&state, &user.id, &id, Some(DeviceKind::User)).await?;
    let removal = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.revoke_device(device.id).await })
        .await??;
    let removed = removal
        .workspaces
        .into_iter()
        .map(|workspace| workspace.id)
        .collect();
    Ok(Json(RevokedDevice { removed }))
}

/// A directory of a paired device, its home when the query names none,
/// with the Host's number before it listed, as the folder dialog's watch
/// compares it (`web-api.md` § File watch).
pub(super) async fn browse(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    QueryParams(query): QueryParams<DeviceDirectoryQuery>,
) -> Result<axum::response::Response, ApiError> {
    let device = owned_device(&state, &user.id, &id, Some(DeviceKind::User)).await?;
    let requested = query.path.map(|path| path.as_str().to_owned());
    let listed = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move {
            let host = shard
                .devices()
                .device_access(&device.id)
                .ok_or_else(ApiError::device_offline)?;
            let home = shard.devices().home(&device.id);
            let path = requested
                .or_else(|| home.clone())
                .ok_or_else(|| ApiError::invalid_query("Missing path query parameter"))?;
            let (entries, read_at) = browse_directory(&host, &path)
                .await
                .map_err(device_fs_error)?;
            let directory = Directory {
                path,
                home,
                entries,
            };
            Ok::<_, ApiError>((directory, read_at))
        })
        .await??;
    let (directory, read_at) = listed;
    Ok(super::files::read_answer(&read_at, Json(directory)))
}

/// `GET /devices/:id/workflows?directory=`: the Workflow menu's list for a
/// new conversation that works in `directory` of a paired device, before
/// its first message gives it a record (`subagents.md` § Workflows of a
/// project), with the Host's number before the read.
pub(super) async fn workflows(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    query: Result<QueryParams<WorkflowsQuery>, ApiError>,
) -> Result<axum::response::Response, ApiError> {
    // A Cloud project's are read as a paired device's, through device
    // access, which wakes no stopped Cloud.
    let device = owned_device(&state, &user.id, &id, None).await?;
    // A directory that is not an absolute path is the request's fault, as a
    // body's would be (`web-api.md` § Workflows of a directory).
    let QueryParams(query) = query.map_err(|error| ApiError::invalid_body(error.message().to_owned()))?;
    let (workflows, read_at) = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move {
            shard
                .device_workflows(&device.id, query.directory.as_str())
                .await
                .map_err(|refusal| match refusal {
                    DeviceWorkflowsRefusal::Offline => ApiError::device_offline(),
                    DeviceWorkflowsRefusal::NoDirectory => ApiError::new(
                        StatusCode::NOT_FOUND,
                        ErrorCode::FsNotFound,
                        "No such directory on the device",
                    ),
                    DeviceWorkflowsRefusal::Host(error) => device_fs_error(error),
                })
        })
        .await??;
    Ok(super::files::read_answer(&read_at, Json(workflows)))
}

/// Makes a directory, with its parents, on a paired device.
pub(super) async fn make_directory(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(CreateDeviceDirectory { path }): JsonBody<CreateDeviceDirectory>,
) -> Result<(StatusCode, Json<CreatedDirectory>), ApiError> {
    let device = owned_device(&state, &user.id, &id, Some(DeviceKind::User)).await?;
    let path = path.as_str().to_owned();
    let made = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move {
            let host = shard
                .devices()
                .device_access(&device.id)
                .ok_or_else(ApiError::device_offline)?;
            let recursive = MkdirOptions { recursive: true };
            demi_host_interface::HostFs::mkdir(&host, &path, recursive)
                .await
                .map_err(device_fs_error)?;
            Ok::<_, ApiError>(CreatedDirectory { path })
        })
        .await??;
    Ok((StatusCode::CREATED, Json(made)))
}

/// Lines of a device's Host log, the Cloud's included.
pub(super) async fn log(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    QueryParams(query): QueryParams<DeviceLogQuery>,
) -> Result<Json<DeviceLog>, ApiError> {
    let device = owned_device(&state, &user.id, &id, None).await?;
    let page = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move {
            let host = shard
                .devices()
                .device_access(&device.id)
                .ok_or_else(ApiError::device_offline)?;
            let source = query.source.as_ref().map(|source| source.as_str());
            host.read_log(query.since, query.limit.get(), source)
                .await
                .map_err(|error| {
                    if error.kind == HostErrorKind::Offline {
                        ApiError::device_offline()
                    } else {
                        ApiError::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            ErrorCode::LogUnreadable,
                            error.message,
                        )
                    }
                })
        })
        .await??;
    let lines = page
        .lines
        .into_iter()
        .map(|line| {
            Ok(DeviceLogLine {
                at: demi_shared_types::Timestamp::from_millisecond(line.at.0).map_err(|error| {
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        ErrorCode::LogUnreadable,
                        error.to_string(),
                    )
                })?,
                source: line.source,
                conversation_id: line.conversation_id,
                text: line.text,
            })
        })
        .collect::<Result<_, ApiError>>()?;
    Ok(Json(DeviceLog {
        lines,
        next: page.next,
    }))
}

/// The caller's device `id`, of `kind` when one is named; 404 otherwise.
pub(super) async fn owned_device(
    state: &AppState,
    user: &demi_web_api_protocol::ids::UserId,
    id: &str,
    kind: Option<DeviceKind>,
) -> Result<DeviceRecord, ApiError> {
    let id = DeviceId::try_from(id).map_err(|_| device_not_found())?;
    let device = state
        .services
        .control
        .device(id)
        .await?
        .ok_or_else(device_not_found)?;
    if !device.is_host_of(user) || kind.is_some_and(|kind| kind != device.kind) {
        return Err(device_not_found());
    }
    Ok(device)
}

fn device_not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        ErrorCode::DeviceNotFound,
        "No such device",
    )
}

/// A device route's filesystem failure: nothing at the path answers 404,
/// anything else the device refused 400.
fn device_fs_error(error: HostError) -> ApiError {
    match error.kind {
        HostErrorKind::Offline => ApiError::device_offline(),
        _ if error.code() == Some("ENOENT") => {
            let gone = ApiError::new(StatusCode::NOT_FOUND, ErrorCode::FsError, error.message);
            // Nothing there is an answer, read at a number like any other.
            match error.read_at {
                Some(read_at) => gone.with_read_at(read_at),
                None => gone,
            }
        }
        _ => ApiError::new(StatusCode::BAD_REQUEST, ErrorCode::FsError, error.message),
    }
}
