//! `GET /conversations/:id/workflows`
//! (`crates/backend-user-shard/src/conversation/workflows.rs`): the Workflow
//! menu's list as the backend stored it at its last read, with when, answered
//! at once; unknown before its first read. The conversation's metadata carries
//! its revision, so a page reads it again when it changes.

use axum::Json;
use axum::extract::{Path, State};
use demi_web_api_protocol::workflows::KeptWorkflowList;

use super::AppState;
use super::conversations::owned;
use super::error::ApiError;
use super::gate::AuthUser;

/// `GET /conversations/:id/workflows`.
pub(super) async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<KeptWorkflowList>, ApiError> {
    let record = owned(&state.services, &user.id, &id).await?;
    let workflows = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move {
            // The kept refusal stays on the shard's thread; its answer leaves it.
            shard.shown_workflows(&record.id).await.map_err(|error| ApiError::from(&*error))
        })
        .await??;
    Ok(Json(workflows))
}
