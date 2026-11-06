//! A conversation's side questions, each in the user's core.
//!
//! Asking one under the id the page chose, a follow-up, a Retry, a Stop, a
//! deletion and folding its card. Each is an agent of the conversation, which
//! a page lists from the conversation's agents; its answers are its
//! transcript, read with `GET /conversations/:id/agents/:node/transcript` and
//! followed on the conversation's socket.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_core::conversation::side_questions::SideRefusal;
use demi_shared_types::NodeId;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::side_questions::{
    AskRequest, SideFollowUp, SideQuestionAnswer, SideQuestionPatch,
};

use crate::api::Api;
use crate::body::JsonBody;
use crate::error::ApiError;
use crate::caller::Principal;

/// `POST /conversations/:id/side-questions { id, blockId, start, end,
/// quote, question }`: the side question, 201; the same request again,
/// 200.
pub(crate) async fn ask(
    State(state): State<Api>,
    Principal(user): Principal,
    Path(id): Path<String>,
    JsonBody(request): JsonBody<AskRequest>,
) -> Result<(StatusCode, Json<SideQuestionAnswer>), ApiError> {
    let id = conversation(id)?;
    let side = side_id(request.id.clone())?;
    let (side_question, created) = state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.ask(id, request).await })
        .await?
        .map_err(refused)?;
    let status = if created { StatusCode::CREATED } else { StatusCode::OK };
    Ok((status, Json(SideQuestionAnswer { id: side, side_question })))
}

/// `POST /conversations/:id/side-questions/:side/messages { content }`: 204
/// once the side question took the follow-up.
pub(crate) async fn follow_up(
    State(state): State<Api>,
    Principal(user): Principal,
    Path((id, side)): Path<(String, String)>,
    JsonBody(SideFollowUp { content }): JsonBody<SideFollowUp>,
) -> Result<StatusCode, ApiError> {
    let id = conversation(id)?;
    let side = side_id(side)?;
    state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.ask_more(id, side, content).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /conversations/:id/side-questions/:side/retry`: 204 once its
/// failed turn runs again.
pub(crate) async fn retry(
    State(state): State<Api>,
    Principal(user): Principal,
    Path((id, side)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let id = conversation(id)?;
    let side = side_id(side)?;
    state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.retry_side(id, side).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /conversations/:id/side-questions/:side/stop`: 204 once its turn
/// and commands are stopped, or when nothing ran; 404 when the conversation
/// has no such side question.
pub(crate) async fn stop(
    State(state): State<Api>,
    Principal(user): Principal,
    Path((id, side)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let id = conversation(id)?;
    let side = side_id(side)?;
    state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.stop_side(id, side).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /conversations/:id/side-questions/:side`: 204 once the side
/// question is stopped and gone with its card; 404 when the conversation has
/// no such side question.
pub(crate) async fn delete(
    State(state): State<Api>,
    Principal(user): Principal,
    Path((id, side)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let id = conversation(id)?;
    let side = side_id(side)?;
    state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.delete_side(id, side).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `PATCH /conversations/:id/side-questions/:side { folded }`: the side
/// question as it is now.
pub(crate) async fn fold(
    State(state): State<Api>,
    Principal(user): Principal,
    Path((id, side)): Path<(String, String)>,
    JsonBody(SideQuestionPatch { folded }): JsonBody<SideQuestionPatch>,
) -> Result<Json<SideQuestionAnswer>, ApiError> {
    let id = conversation(id)?;
    let side = side_id(side)?;
    let folding = side.clone();
    let side_question = state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.fold_side(id, folding, folded).await })
        .await?
        .map_err(refused)?;
    Ok(Json(SideQuestionAnswer { id: side, side_question }))
}

fn conversation(id: String) -> Result<ConversationId, ApiError> {
    ConversationId::try_from(id).map_err(|_| refused(SideRefusal::ConversationNotFound))
}

fn side_id(side: String) -> Result<NodeId, ApiError> {
    NodeId::try_from(side).map_err(|_| refused(SideRefusal::NotFound))
}

fn refused(refusal: SideRefusal) -> ApiError {
    let message = refusal.to_string();
    let (status, code) = match refusal {
        SideRefusal::ConversationNotFound => (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound),
        SideRefusal::Archived => (StatusCode::CONFLICT, ErrorCode::ConversationArchived),
        SideRefusal::NotFound => (StatusCode::NOT_FOUND, ErrorCode::SideQuestionNotFound),
        SideRefusal::Conflict => (StatusCode::CONFLICT, ErrorCode::SideQuestionConflict),
        SideRefusal::Target(_) => (StatusCode::BAD_REQUEST, ErrorCode::InvalidSideQuestionTarget),
        SideRefusal::Refused(_) => (StatusCode::CONFLICT, ErrorCode::SideQuestionRefused),
        SideRefusal::Storage(error) => return error.into(),
        SideRefusal::Failed(_) => (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::InternalError),
    };
    ApiError::new(status, code, message)
}
