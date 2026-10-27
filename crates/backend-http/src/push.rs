//! `/push/key` and `/push/subscription`.
//!
//! They serve the backend's VAPID public key, and the caller's session's
//! subscription, which the backend keeps and removes. Neither reaches another
//! session's.
//!
//! `/conversations/:id/notifications-card/dismiss` dismisses the card that
//! asks the user to turn notifications on, which a subscription that takes
//! the agent's notifications takes from every conversation.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_accounts::push::PushError;
use demi_backend_page_sync::Part;
use demi_backend_user_shard::services::Services;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::push::{PushKey, PushSubscriptionBody};

use super::body::JsonBody;
use super::conversations::owned;
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
    let agent_notifies = body.events.agent_notifies;
    services
        .push
        .subscribe(user.id.clone(), session.id, body)
        .await
        .map_err(|error| match error {
            PushError::Storage(error) => ApiError::from(error),
            PushError::Taken => ApiError::new(StatusCode::CONFLICT, ErrorCode::PushEndpointTaken, error.to_string()),
            PushError::NotPublic(_) => ApiError::invalid_body(error.to_string()),
        })?;
    // Each session says whether its browser has notifications on.
    services.sync.mark(&user.id, Part::Sessions);
    if agent_notifies {
        for conversation in services.control.clear_notifications_cards(user.id.clone()).await? {
            services.sync.mark(&user.id, Part::Metadata(conversation));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /conversations/:id/notifications-card/dismiss`: the conversation
/// never shows the card again.
pub(super) async fn dismiss_card(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let record = owned(&services, &user.id, &id).await?;
    if services.control.dismiss_notifications_card(record.id.clone()).await? {
        services.sync.mark(&user.id, Part::Metadata(record.id));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /push/subscription`: the caller's session's, if it has one.
pub(super) async fn unsubscribe(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    session: CurrentSession,
) -> Result<StatusCode, ApiError> {
    services.push.unsubscribe(session.id).await?;
    services.sync.mark(&user.id, Part::Sessions);
    Ok(StatusCode::NO_CONTENT)
}
