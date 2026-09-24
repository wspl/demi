//! `/api/devices` (`web-api.md` § Workspaces, devices, and attached hosts,
//! § Device files and remote references, § Device log): the caller's
//! devices, pairing, revocation, browsing a paired device's directories to
//! choose a target, and a Host's log. Browsing and the log reach the device
//! through device access, which wakes nothing: a device whose runner is not
//! connected answers 409 `device_offline`.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_shell::{HostError, HostErrorKind, MkdirOptions};
use demi_web_api::devices::{Claim, ClaimedDevice, DeviceKind, DeviceLog, DeviceLogLine, DeviceLogQuery, Devices};
use demi_web_api::error::ErrorCode;
use demi_web_api::files::{CreateDeviceDirectory, CreatedDirectory, DeviceDirectoryQuery, Directory};
use demi_web_api::ids::DeviceId;

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;
use super::query::QueryParams;
use crate::auth::sessions::TokenHash;
use crate::runner::codes::{ClaimCode, new_device_token};
use crate::runner::files::browse_directory;
use crate::storage::devices::DeviceRecord;

pub(super) async fn list(State(state): State<AppState>, AuthUser(user): AuthUser) -> Result<Json<Devices>, ApiError> {
    let devices = state
        .shards
        .of(&user.id)
        .call(|shard, _| async move { shard.device_list().await })
        .await??;
    let devices = devices.into_iter().filter(|device| device.kind == DeviceKind::User).collect();
    Ok(Json(Devices { devices }))
}

/// Pairs the runner waiting under the code with the caller: the device is
/// made, its token goes to that runner alone, and the runner's socket moves
/// into the caller's shard.
pub(super) async fn claim(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    JsonBody(Claim { code }): JsonBody<Claim>,
) -> Result<(StatusCode, Json<ClaimedDevice>), ApiError> {
    let services = &state.services;
    if !services.claims.attempt(&user.id) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::RateLimited,
            "Too many claim attempts",
        ));
    }
    let pending = ClaimCode::parse(&code)
        .and_then(|code| services.claims.take(&code))
        .ok_or_else(invalid_code)?;
    let token = new_device_token();
    let device = services
        .control
        .create_device(
            user.id.clone(),
            pending.runner.name.clone(),
            pending.runner.platform.clone(),
            TokenHash::of(token.expose()),
        )
        .await?;
    let id = device.id.clone();
    match pending.grant(device, token).await {
        Ok(device) => Ok((StatusCode::CREATED, Json(ClaimedDevice { device }))),
        Err(_) => {
            // The runner went away before it held its token: the device it
            // would have been goes too.
            services.control.delete_device(id).await?;
            Err(invalid_code())
        }
    }
}

fn invalid_code() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, ErrorCode::InvalidCode, "Unknown or expired pairing code")
}

/// Revokes a paired device: it stays while workspaces point at it.
pub(super) async fn revoke(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let device = owned_device(&state, &user.id, &id, Some(DeviceKind::User)).await?;
    let workspaces = state.services.control.workspaces_on_device(device.id.clone()).await?;
    if workspaces > 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::DeviceInUse,
            format!("{workspaces} workspace(s) still point at this device"),
        ));
    }
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.revoke_device(device.id).await })
        .await??;
    Ok(StatusCode::NO_CONTENT)
}

/// A directory of a paired device, its home when the query names none.
pub(super) async fn browse(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    QueryParams(query): QueryParams<DeviceDirectoryQuery>,
) -> Result<Json<Directory>, ApiError> {
    let device = owned_device(&state, &user.id, &id, Some(DeviceKind::User)).await?;
    let requested = query.path.map(|path| path.as_str().to_owned());
    let listed = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move {
            let host = shard.devices().device_access(&device.id).ok_or_else(device_offline)?;
            let home = shard.devices().home(&device.id);
            let path = requested
                .or_else(|| home.clone())
                .ok_or_else(|| ApiError::invalid_query("Missing path query parameter"))?;
            let entries = browse_directory(&host, &path).await.map_err(device_fs_error)?;
            Ok::<_, ApiError>(Directory { path, home, entries })
        })
        .await??;
    Ok(Json(listed))
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
            let host = shard.devices().device_access(&device.id).ok_or_else(device_offline)?;
            let recursive = MkdirOptions { recursive: true };
            demi_shell::HostFs::mkdir(&host, &path, recursive)
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
            let host = shard.devices().device_access(&device.id).ok_or_else(device_offline)?;
            let source = query.source.as_ref().map(|source| source.as_str());
            host.read_log(query.since, query.limit.get(), source).await.map_err(|error| {
                if error.kind == HostErrorKind::Offline {
                    device_offline()
                } else {
                    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::LogUnreadable, error.message)
                }
            })
        })
        .await??;
    let lines = page
        .lines
        .into_iter()
        .map(|line| {
            Ok(DeviceLogLine {
                at: demi_core::Timestamp::from_millisecond(line.at.0)
                    .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::LogUnreadable, error.to_string()))?,
                source: line.source,
                conversation_id: line.conversation_id,
                text: line.text,
            })
        })
        .collect::<Result<_, ApiError>>()?;
    Ok(Json(DeviceLog { lines, next: page.next }))
}

/// The caller's device `id`, of `kind` when one is named; 404 otherwise.
async fn owned_device(
    state: &AppState,
    user: &demi_web_api::ids::UserId,
    id: &str,
    kind: Option<DeviceKind>,
) -> Result<DeviceRecord, ApiError> {
    let not_found = || ApiError::new(StatusCode::NOT_FOUND, ErrorCode::DeviceNotFound, "No such device");
    let id = DeviceId::try_from(id).map_err(|_| not_found())?;
    let device = state.services.control.device(id).await?.ok_or_else(not_found)?;
    if device.user != *user || kind.is_some_and(|kind| kind != device.kind) {
        return Err(not_found());
    }
    Ok(device)
}

pub(super) fn device_offline() -> ApiError {
    ApiError::new(StatusCode::CONFLICT, ErrorCode::DeviceOffline, "Device is offline")
}

/// A device route's filesystem failure: nothing at the path answers 404,
/// anything else the device refused 400.
fn device_fs_error(error: HostError) -> ApiError {
    match error.kind {
        HostErrorKind::Offline => device_offline(),
        _ if error.code() == Some("ENOENT") => ApiError::new(StatusCode::NOT_FOUND, ErrorCode::FsError, error.message),
        _ => ApiError::new(StatusCode::BAD_REQUEST, ErrorCode::FsError, error.message),
    }
}
