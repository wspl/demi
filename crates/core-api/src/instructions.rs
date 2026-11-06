//! The caller's personal instructions.
//!
//! The edge checks the text. The caller's core writes it, and the change
//! reaches the caller's pages as the `instructions` part.

use axum::extract::State;
use axum::http::StatusCode;
use demi_core::instructions::personal_instructions;
use axum::Json;
use demi_web_api_protocol::instructions::{InstructionsAnswer, InstructionsBody};

use crate::api::Api;
use crate::body::JsonBody;
use crate::error::ApiError;
use crate::caller::Principal;

/// `GET /settings/instructions`: the caller's instructions, empty when they
/// have none.
pub(crate) async fn read(
    State(state): State<Api>,
    Principal(user): Principal,
) -> Result<Json<InstructionsAnswer>, ApiError> {
    let instructions = state.store(&user).await?.instructions().await?;
    Ok(Json(InstructionsAnswer { instructions }))
}

/// `PUT /instructions { text }`.
pub(crate) async fn replace(
    State(state): State<Api>,
    Principal(user): Principal,
    JsonBody(InstructionsBody { text }): JsonBody<InstructionsBody>,
) -> Result<StatusCode, ApiError> {
    let text = personal_instructions(text);
    state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.set_instructions(text).await })
        .await??;
    Ok(StatusCode::NO_CONTENT)
}
