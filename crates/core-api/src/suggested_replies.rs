//! `POST /conversations/:id/suggested-replies/used`: the user took a suggested
//! reply, which the backend keeps to measure the replies by.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_core::services::CoreServices;
use demi_web_api_protocol::conversations::SuggestedReplyUsed;

use crate::body::JsonBody;
use crate::conversations::owned;
use crate::error::ApiError;
use crate::caller::Principal;

pub(crate) async fn used(
    State(services): State<Arc<CoreServices>>,
    Principal(user): Principal,
    Path(id): Path<String>,
    body: Result<JsonBody<SuggestedReplyUsed>, ApiError>,
) -> Result<StatusCode, ApiError> {
    let record = owned(&services, &user, &id).await?;
    let JsonBody(used) = body?;
    services.vault.stores().store_of(&user).await?.suggested_reply_used(record.id, used).await?;
    Ok(StatusCode::NO_CONTENT)
}
