//! The caller's personal instructions.
//!
//! The edge checks the text. The caller's shard writes it, and the change
//! reaches the caller's pages as the `instructions` part.

use axum::extract::State;
use axum::http::StatusCode;
use demi_backend_accounts::instructions::personal_instructions;
use axum::Json;
use demi_web_api_protocol::instructions::{InstructionsAnswer, InstructionsBody};

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;

/// `GET /settings/instructions`: the caller's instructions, empty when they
/// have none.
pub(super) async fn read(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<InstructionsAnswer>, ApiError> {
    let instructions = state.services.control.instructions(user.id).await?;
    Ok(Json(InstructionsAnswer { instructions }))
}

/// `PUT /instructions { text }`.
pub(super) async fn replace(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    JsonBody(InstructionsBody { text }): JsonBody<InstructionsBody>,
) -> Result<StatusCode, ApiError> {
    let text = personal_instructions(text);
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.set_instructions(text).await })
        .await??;
    Ok(StatusCode::NO_CONTENT)
}
