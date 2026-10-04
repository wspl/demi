//! A conversation's permission routes (`web-api.md` § Conversation
//! permissions): the read of its undecided requests and its grants, a
//! decision on a request, and the revocation of a grant, each in the user's
//! shard. An archived conversation reads its grants and refuses the rest.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use demi_backend_permissions::{PermissionError, decide as decide_request, read as read_all, revoke as revoke_grant};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::permissions::{
    ConversationPermissions, DecidePermission, PermissionRequestId,
};

use super::AppState;
use super::body::JsonBody;
use super::conversations::owned;
use super::error::ApiError;
use super::gate::AuthUser;

/// `GET /conversations/:id/permissions`.
pub(super) async fn read(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ConversationPermissions>, ApiError> {
    let record = owned(&state.services, &user.id, &id).await?;
    let conversation = record.id;
    let permissions = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move {
            read_all(shard.permission_shard(), &conversation).await
        })
        .await?
        .map_err(refused)?;
    Ok(Json(permissions))
}

/// `POST /conversations/:id/permissions/requests/:request`.
pub(super) async fn decide(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, request)): Path<(String, String)>,
    body: Result<JsonBody<DecidePermission>, ApiError>,
) -> Result<StatusCode, ApiError> {
    let record = owned(&state.services, &user.id, &id).await?;
    let JsonBody(DecidePermission { decision }) = body?;
    let request = PermissionRequestId::try_from(request).map_err(|_| refused(PermissionError::NotFound))?;
    let conversation = record.id;
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move {
            decide_request(shard.permission_shard(), &conversation, request, decision).await
        })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /conversations/:id/permissions/grants/:category`.
pub(super) async fn revoke(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, category)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let record = owned(&state.services, &user.id, &id).await?;
    let conversation = record.id;
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move {
            revoke_grant(shard.permission_shard(), &conversation, category).await
        })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

fn refused(error: PermissionError) -> ApiError {
    let message = error.to_string();
    match error {
        PermissionError::Archived => ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::ConversationArchived,
            message,
        ),
        PermissionError::NotFound => ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::PermissionRequestNotFound,
            message,
        ),
        PermissionError::Storage(error) => error.into(),
    }
}
