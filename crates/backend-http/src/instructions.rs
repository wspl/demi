//! The caller's personal instructions (`web-api.md` § Instructions). The
//! edge checks the text; the caller's shard writes it, and the change
//! reaches the caller's pages as the `instructions` part.

use axum::extract::State;
use axum::http::StatusCode;
use demi_backend_accounts::instructions::personal_instructions;
use demi_web_api_protocol::instructions::InstructionsBody;

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;

/// `PUT /instructions { text }`.
pub(super) async fn replace(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    JsonBody(InstructionsBody { text }): JsonBody<InstructionsBody>,
) -> Result<StatusCode, ApiError> {
    let text = personal_instructions(text).map_err(|error| ApiError::invalid_body(error.to_string()))?;
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.set_instructions(text).await })
        .await??;
    Ok(StatusCode::NO_CONTENT)
}
