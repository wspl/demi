//! A conversation's side questions, each in the user's shard.
//!
//! - The list a page reads when the summary's revision passes what it holds.
//! - Asking one under the id the page chose, a follow-up, a Retry and folding
//!   its card.
//!
//! Its answers are its transcript, read with `GET
//! /conversations/:id/transcript?node=` and followed on the conversation's
//! socket.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_user_shard::conversation::side_questions::SideRefusal;
use demi_shared_types::NodeId;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::side_questions::{
    AskRequest, SideFollowUp, SideQuestionAnswer, SideQuestionPatch, SideQuestions,
};

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;

/// `GET /conversations/:id/side-questions`.
pub(super) async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<SideQuestions>, ApiError> {
    let id = conversation(id)?;
    let listed = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.side_questions(id).await })
        .await?
        .map_err(refused)?;
    Ok(Json(listed))
}

/// `POST /conversations/:id/side-questions { id, blockId, start, end,
/// quote, question }`: the side question, 201; the same request again,
/// 200.
pub(super) async fn ask(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(request): JsonBody<AskRequest>,
) -> Result<(StatusCode, Json<SideQuestionAnswer>), ApiError> {
    let id = conversation(id)?;
    let (side_question, created) = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.ask(id, request).await })
        .await?
        .map_err(refused)?;
    let status = if created { StatusCode::CREATED } else { StatusCode::OK };
    Ok((status, Json(SideQuestionAnswer { side_question })))
}

/// `POST /conversations/:id/side-questions/:side/messages { text }`: 204
/// once the side question took the follow-up.
pub(super) async fn follow_up(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, side)): Path<(String, String)>,
    JsonBody(SideFollowUp { text }): JsonBody<SideFollowUp>,
) -> Result<StatusCode, ApiError> {
    let id = conversation(id)?;
    let side = side_id(side)?;
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.ask_more(id, side, text).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /conversations/:id/side-questions/:side/retry`: 204 once its
/// failed turn runs again.
pub(super) async fn retry(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, side)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let id = conversation(id)?;
    let side = side_id(side)?;
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.retry_side(id, side).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `PATCH /conversations/:id/side-questions/:side { folded }`: the side
/// question as it is now.
pub(super) async fn fold(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, side)): Path<(String, String)>,
    JsonBody(SideQuestionPatch { folded }): JsonBody<SideQuestionPatch>,
) -> Result<Json<SideQuestionAnswer>, ApiError> {
    let id = conversation(id)?;
    let side = side_id(side)?;
    let side_question = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.fold_side(id, side, folded).await })
        .await?
        .map_err(refused)?;
    Ok(Json(SideQuestionAnswer { side_question }))
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
