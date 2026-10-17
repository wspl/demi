//! `GET /conversations/:id/workflows` (`subagents.md` § Workflows of a
//! project): the Workflow menu's list, as the backend keeps it with the
//! runner's file watch, with the Host's number before the read that found
//! it, as every answer the file watch keeps current carries.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::Response;
use demi_backend_host_access::access::{HostAccessError, Refusal};
use demi_backend_host_access::plugin_files::ReadFilesError;
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
        .call(move |shard, _| async move { shard.menu_workflows(&record.id, read_after).await })
        .await?
        .map_err(|error| match error {
            // A Host that is not running answers as a stopped one does; the
            // menu's reads are bound to no Host, so none answers moved.
            ReadFilesError::NotRunning | ReadFilesError::Moved => {
                ApiError::from(HostAccessError::Refused(Refusal::Stopped))
            }
            ReadFilesError::Access(error) => ApiError::from(error),
        })?;
    Ok(read_answer(&read_at, Json(workflows)))
}
