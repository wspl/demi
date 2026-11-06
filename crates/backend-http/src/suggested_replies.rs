//! `POST /conversations/:id/suggested-replies/used`: the user took a suggested
//! reply, which the backend keeps to measure the replies by.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_user_shard::services::Services;
use demi_web_api_protocol::conversations::SuggestedReplyUsed;

use super::body::JsonBody;
use super::conversations::owned;
use super::error::ApiError;
use super::gate::AuthUser;

pub(super) async fn used(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    body: Result<JsonBody<SuggestedReplyUsed>, ApiError>,
) -> Result<StatusCode, ApiError> {
    let record = owned(&services, &user.id, &id).await?;
    let JsonBody(used) = body?;
    services.control.suggested_reply_used(record.id, used).await?;
    Ok(StatusCode::NO_CONTENT)
}
