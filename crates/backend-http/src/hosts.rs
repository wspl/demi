//! A conversation's attached hosts
//! (`crates/backend-database/src/conversation_index.rs`).
//!
//! The route is a detach; the conversation's metadata lists the hosts. A detach
//! is a transition and reaches the conversation's nodes at their next context
//! block. The conversation's agents attach the devices.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_database::conversation_index::{ConversationChange, RecordChange};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId};

use super::AppState;
use super::error::ApiError;
use super::gate::AuthUser;
use demi_backend_host_access::transition::ChangeRefusal;

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
async fn change(
    state: &AppState,
    user: &UserId,
    id: &ConversationId,
    change: ConversationChange,
) -> Result<(), ApiError> {
    let id = id.clone();
    state
        .shards
        .of(user)
        .call(move |shard, _| async move { shard.transition(&id, change).await })
        .await?
        .map_err(refused)
}

fn conversation_id(id: &str) -> Result<ConversationId, ApiError> {
    ConversationId::try_from(id).map_err(|_| refused(ChangeRefusal::NotFound))
}

pub(super) fn refused(refusal: ChangeRefusal) -> ApiError {
    match refusal {
        ChangeRefusal::Storage(error) => error.into(),
        refusal => {
            let (code, status) = refusal.code();
            let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            ApiError::new(status, code, refusal.to_string())
        }
    }
}
