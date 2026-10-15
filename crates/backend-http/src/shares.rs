//! Sharing (`web-api.md` § Sharing): the owner's routes, which make, take
//! again and remove a conversation's share, and the public routes under
//! `/api/shares/:token`, which take no session and ignore one: the token is
//! the only key, and every miss answers the same 404 `share_not_found`, so
//! an answer tells nothing of what exists. A public route finds the share's
//! record, then reads its snapshot on the owner's shard, the conversation's
//! history reads (`history.rs`) answering it.

use axum::Json;
use axum::extract::{Path, RawQuery, State};
use axum::http::header::{CACHE_CONTROL, REFERRER_POLICY};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::Response;
use demi_backend_database::shares::ShareRecord;
use demi_backend_user_shard::conversation::shares::{ShareBlobError, ShareRefusal};
use demi_backend_user_shard::services::Services;
use demi_shared_types::{BlobRef, BlockId};
use demi_web_api_protocol::conversations::{TranscriptPage, WholeBlock};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{ConversationId, ShareToken};
use demi_web_api_protocol::shares::{
    ShareAnswer, ShareNodeQuery, ShareTarget, ShareTranscriptQuery, SharedAgent, SharedConversation,
};
use demi_web_api_protocol::attachments::ConversationAttachment;
use demi_backend_database::conversation_attachments::AttachmentNumber;

use super::AppState;
use super::blobs::blob_answer;
use super::conversations::attachment_answer;
use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;
use super::history::{block_answer, page_answer};
use super::query::QueryParams;

/// `POST /conversations/:id/shares { kind, blockId? }`: the conversation's
/// share of that target, the one it has or a new snapshot.
pub(super) async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(target): JsonBody<ShareTarget>,
) -> Result<Json<ShareAnswer>, ApiError> {
    let conversation = ConversationId::try_from(id).map_err(|_| refused(ShareRefusal::NoConversation))?;
    let share = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.share(conversation, target).await })
        .await?
        .map_err(refused)?;
    Ok(Json(answer(&state.services, share)))
}

/// `PUT /conversations/:id/shares/:token`: the conversation's snapshot taken
/// again under the same token.
pub(super) async fn update(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, token)): Path<(String, String)>,
) -> Result<Json<ShareAnswer>, ApiError> {
    let (conversation, token) = owner_path(id, token)?;
    let share = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.update_share(conversation, token).await })
        .await?
        .map_err(refused)?;
    Ok(Json(answer(&state.services, share)))
}

/// `DELETE /conversations/:id/shares/:token`: the share removed.
pub(super) async fn remove(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, token)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let (conversation, token) = owner_path(id, token)?;
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.stop_sharing(conversation, token).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

fn owner_path(id: String, token: String) -> Result<(ConversationId, ShareToken), ApiError> {
    let conversation = ConversationId::try_from(id).map_err(|_| refused(ShareRefusal::NoConversation))?;
    let token = ShareToken::try_from(token).map_err(|_| not_found())?;
    Ok((conversation, token))
}

fn answer(services: &Services, share: ShareRecord) -> ShareAnswer {
    let public = services
        .public_url
        .get()
        .expect("the backend listens before it serves a request")
        .url();
    let url = public
        .join(&format!("s/{}", share.token))
        .expect("a token is URL-safe base64");
    ShareAnswer::new(share.token, url.into(), share.target, share.taken_at)
}

fn refused(refusal: ShareRefusal) -> ApiError {
    match refusal {
        ShareRefusal::NoConversation => ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::ConversationNotFound,
            "No such conversation",
        ),
        ShareRefusal::NoShare => not_found(),
        ShareRefusal::TargetInvalid => ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::ShareTargetInvalid,
            refusal.to_string(),
        ),
        ShareRefusal::NotUpdatable => ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::ShareNotUpdatable,
            refusal.to_string(),
        ),
        ShareRefusal::Storage(error) => error.into(),
    }
}

/// The one answer of every miss of a share's routes.
fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, ErrorCode::ShareNotFound, "This shared link does not exist")
}

/// The share `token` names.
async fn found(services: &Services, token: String) -> Result<ShareRecord, ApiError> {
    let token = ShareToken::try_from(token).map_err(|_| not_found())?;
    services.control.share(token).await?.ok_or_else(not_found)
}

/// `GET /shares/:token`: what the shared page shows above the snapshot.
pub(super) async fn read(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<Json<SharedConversation>, ApiError> {
    let share = found(&state.services, token).await?;
    let (kind, title, taken_at) = (share.target.kind(), share.title.clone(), share.taken_at);
    let owner = share.owner.clone();
    let agents = state
        .shards
        .of(&owner)
        .call(move |shard, _| async move { shard.shared_agents(&share).await })
        .await?
        .map_err(ApiError::conversation_unreadable)?;
    Ok(Json(SharedConversation {
        kind,
        title,
        taken_at,
        agents: agents
            .into_iter()
            .map(|agent| SharedAgent {
                number: agent.number,
                description: agent.description,
                profile: agent.profile,
            })
            .collect(),
    }))
}

/// `GET /shares/:token/attachments/:attachment`: an attachment the
/// snapshot's text names, as the conversation's attachment route answers
/// it.
pub(super) async fn attachment(
    State(state): State<AppState>,
    Path((token, attachment)): Path<(String, String)>,
) -> Result<Json<ConversationAttachment>, ApiError> {
    let share = found(&state.services, token).await?;
    let number: AttachmentNumber = attachment.parse().map_err(|_| not_found())?;
    let owner = share.owner.clone();
    let row = state
        .shards
        .of(&owner)
        .call(move |shard, _| async move { shard.shared_attachment(&share, number).await })
        .await?
        .map_err(ApiError::conversation_unreadable)?
        .ok_or_else(not_found)?;
    Ok(Json(attachment_answer(row)))
}

/// `GET /shares/:token/transcript`: a page of the snapshot, as a
/// conversation's page is answered, without failure facts, which are read
/// live, and with no instructions, which a snapshot does not hold.
pub(super) async fn transcript(
    State(state): State<AppState>,
    Path(token): Path<String>,
    QueryParams(query): QueryParams<ShareTranscriptQuery>,
) -> Result<Json<TranscriptPage>, ApiError> {
    let share = found(&state.services, token).await?;
    let owner = share.owner.clone();
    let read = state
        .shards
        .of(&owner)
        .call(move |shard, _| async move { shard.share_page(&share, query).await })
        .await?
        .map_err(ApiError::conversation_unreadable)?;
    // A conversation without a database holds no snapshot.
    let read = read.ok_or_else(not_found)?;
    let page = page_answer(Some(read), not_found, not_found)?;
    Ok(Json(page))
}

/// `GET /shares/:token/transcript/blocks/:blockId`: a block of the snapshot
/// whole, as the conversation's whole-block read answers it.
pub(super) async fn block(
    State(state): State<AppState>,
    Path((token, block)): Path<(String, String)>,
    QueryParams(query): QueryParams<ShareNodeQuery>,
) -> Result<Json<WholeBlock>, ApiError> {
    let share = found(&state.services, token).await?;
    let block = BlockId::try_from(block).map_err(|_| not_found())?;
    let owner = share.owner.clone();
    let read = state
        .shards
        .of(&owner)
        .call(move |shard, _| async move {
            shard.share_block(&share, query.node.unwrap_or(0), block).await
        })
        .await?
        .map_err(ApiError::conversation_unreadable)?;
    Ok(Json(WholeBlock {
        block: block_answer(read, not_found, not_found)?,
        failures: None,
    }))
}

/// `GET /shares/:token/blobs/:sha256?type=`: a blob the snapshot shows.
pub(super) async fn blob(
    State(state): State<AppState>,
    Path((token, name)): Path<(String, String)>,
    RawQuery(query): RawQuery,
    request: HeaderMap,
) -> Result<Response, ApiError> {
    let share = found(&state.services, token).await?;
    let blob = BlobRef::try_from(name).map_err(|_| not_found())?;
    let owner = share.owner.clone();
    let bytes = state
        .shards
        .of(&owner)
        .call(move |shard, _| async move { shard.share_blob(&share, blob).await })
        .await?
        .map_err(|error| match error {
            ShareBlobError::Storage(error) => ApiError::conversation_unreadable(error),
            ShareBlobError::Objects(error) => error.into(),
        })?
        .ok_or_else(not_found)?;
    Ok(blob_answer(&bytes, query.as_deref(), &request))
}

/// What every answer of the public routes carries (`web-api.md` § Sharing):
/// not indexed, no referrer that would carry the token, and kept by no
/// cache, so a removed share is not served from one.
pub(super) async fn public_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        HeaderName::from_static("x-robots-tag"),
        HeaderValue::from_static("noindex, nofollow"),
    );
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    response
}
