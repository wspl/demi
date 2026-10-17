//! `/push/key` and `/push/subscription` (`web-api-settings.md` § Push
//! notifications): the backend's VAPID public key, and the caller's
//! session's subscription, which it keeps and removes. Neither reaches
//! another session's.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use demi_backend_accounts::push::PushError;
use demi_backend_user_shard::services::Services;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::push::{PushKey, PushSubscriptionBody};

use super::body::JsonBody;
use super::error::ApiError;
use super::gate::{AuthUser, CurrentSession};

/// `GET /push/key`.
pub(super) async fn key(State(services): State<Arc<Services>>, AuthUser(_): AuthUser) -> Json<PushKey> {
    Json(PushKey {
        public_key: services.push.public_key(),
    })
}

/// `PUT /push/subscription`: kept for the caller's session, whose end
/// removes it.
pub(super) async fn subscribe(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    session: CurrentSession,
    JsonBody(body): JsonBody<PushSubscriptionBody>,
) -> Result<StatusCode, ApiError> {
    services
        .push
        .subscribe(user.id, session.id, body)
        .await
        .map_err(|error| match error {
            PushError::Storage(error) => ApiError::from(error),
            PushError::Taken => ApiError::new(StatusCode::CONFLICT, ErrorCode::PushEndpointTaken, error.to_string()),
            PushError::NotPublic(_) => ApiError::invalid_body(error.to_string()),
        })?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /push/subscription`: the caller's session's, if it has one.
pub(super) async fn unsubscribe(
    State(services): State<Arc<Services>>,
    session: CurrentSession,
) -> Result<StatusCode, ApiError> {
    services.push.unsubscribe(session.id).await?;
    Ok(StatusCode::NO_CONTENT)
}
