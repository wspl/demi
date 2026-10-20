//! `GET /conversations/:id/workflows`
//! (`crates/backend-user-shard/src/conversation/workflows.rs`): the Workflow
//! menu's list, as the backend keeps it with the runner's file watch.
//!
//! The answer carries the Host's number before the read that found it, as every
//! answer the file watch keeps current carries. While no search is held, there
//! are no workflows and no number.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use demi_web_api_protocol::workflows::ReadAfterQuery;

use super::AppState;
use super::conversations::owned;
use super::error::ApiError;
use super::files::read_answer;
use super::gate::AuthUser;
use super::query::QueryParams;

/// `GET /conversations/:id/workflows`.
pub(super) async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    QueryParams(ReadAfterQuery { read_after }): QueryParams<ReadAfterQuery>,
) -> Result<Response, ApiError> {
    let record = owned(&state.services, &user.id, &id).await?;
    let (workflows, read_at) = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move {
            // The kept refusal stays on the shard's thread; its answer leaves it.
            shard.menu_workflows(&record.id, read_after).await.map_err(|error| ApiError::from(&*error))
        })
        .await??;
    Ok(match read_at {
        Some(read_at) => read_answer(&read_at, Json(workflows)),
        None => Json(workflows).into_response(),
    })
}
