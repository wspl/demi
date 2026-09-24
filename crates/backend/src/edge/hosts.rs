//! A conversation's attached hosts (`web-api.md` § Workspaces, devices, and
//! attached hosts; `sessions-and-targets.md` § Attached hosts): the list,
//! with each device's connection, an attach of one of the user's devices, a
//! rename, and a detach, which is a transition. Every change reaches the
//! conversation's nodes at their next context block.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_web_api::error::ErrorCode;
use demi_web_api::hosts::{AttachHost, AttachedHost, AttachedHosts, RenameHost};
use demi_web_api::ids::{ConversationId, DeviceId, UserId};

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;
use crate::conversation::transition::ChangeRefusal;
use crate::storage::conversation_index::{AttachedHostRecord, ConversationChange, RecordChange};

pub(super) async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<AttachedHosts>, ApiError> {
    let id = conversation_id(&id)?;
    Ok(Json(hosts(&state, &user.id, id).await?))
}

/// Attaches one of the user's devices other than the main Host; attached
/// already, it stays as it is.
pub(super) async fn attach(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(AttachHost { device_id }): JsonBody<AttachHost>,
) -> Result<(StatusCode, Json<AttachedHosts>), ApiError> {
    let id = conversation_id(&id)?;
    let device = state
        .services
        .control
        .device(device_id)
        .await?
        .filter(|device| device.user == user.id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, ErrorCode::DeviceNotFound, "No such device"))?;
    let host = AttachedHostRecord {
        device: device.id,
        name: device.name,
        cwd: None,
    };
    change(&state, &user.id, &id, RecordChange::Attach(host).into()).await?;
    Ok((StatusCode::CREATED, Json(hosts(&state, &user.id, id).await?)))
}

/// Renames an attached host, uniquely within the conversation.
pub(super) async fn rename(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, device)): Path<(String, String)>,
    JsonBody(RenameHost { name }): JsonBody<RenameHost>,
) -> Result<Json<AttachedHosts>, ApiError> {
    let id = conversation_id(&id)?;
    let device = DeviceId::try_from(device).map_err(|_| refused(ChangeRefusal::NotAttached))?;
    let renamed = RecordChange::Rename {
        device,
        name: name.into_string(),
    };
    change(&state, &user.id, &id, renamed.into()).await?;
    Ok(Json(hosts(&state, &user.id, id).await?))
}

/// Detaches a device, which hears the conversation release; a device that
/// is not attached detaches as nothing.
pub(super) async fn detach(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, device)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let id = conversation_id(&id)?;
    let Ok(device) = DeviceId::try_from(device) else {
        return Ok(StatusCode::NO_CONTENT);
    };
    change(&state, &user.id, &id, RecordChange::Detach(device).into()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Applies `change` through the conversation's transition.
async fn change(state: &AppState, user: &UserId, id: &ConversationId, change: ConversationChange) -> Result<(), ApiError> {
    let id = id.clone();
    state
        .shards
        .of(user)
        .call(move |shard, _| async move { shard.transition(&id, change).await })
        .await?
        .map_err(refused)
}

/// The conversation's attached hosts, each with its device's connection.
async fn hosts(state: &AppState, user: &UserId, id: ConversationId) -> Result<AttachedHosts, ApiError> {
    state
        .shards
        .of(user)
        .call(move |shard, _| async move {
            let record = shard.owned_conversation(&id).await?;
            let listed = shard.services().control.attached_host_listing(record.id).await?;
            let hosts = listed
                .into_iter()
                .map(|(host, attached_at)| AttachedHost {
                    online: shard.devices().online(&host.device),
                    device_id: host.device,
                    name: host.name,
                    cwd: host.cwd,
                    attached_at,
                })
                .collect();
            Ok::<_, ApiError>(AttachedHosts { hosts })
        })
        .await?
}

fn conversation_id(id: &str) -> Result<ConversationId, ApiError> {
    ConversationId::try_from(id).map_err(|_| refused(ChangeRefusal::NotFound))
}

fn refused(refusal: ChangeRefusal) -> ApiError {
    match refusal {
        ChangeRefusal::Storage(error) => error.into(),
        refusal => {
            let (code, status) = refusal.code();
            let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            ApiError::new(status, code, refusal.to_string())
        }
    }
}
