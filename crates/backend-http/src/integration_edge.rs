//! The integration API (`crates/integration-api-protocol/src/lib.rs`): the
//! routes under `/v1` that a program building on Demi calls with its
//! integration's token.
//!
//! The edge is the web app's twin: it authenticates the request, decodes its
//! body into the integration protocol's types and calls the same shard
//! operations the web app's routes reach, and maps their answers back. A
//! request reaches only its integration's own Clouds and conversations; any
//! other id answers 404, as one that is not there.
//!
//! The socket (`GET /v1/socket`) sends the integration every change of its
//! conversations, which the shard hears from their trees
//! (`IntegrationObservers`). It sends nothing from before it opened: a
//! program that connects again reads what it shows again.

use axum::Json;
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::body::Body;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::Response;
use axum::routing::{delete, get, post};
use demi_backend_database::conversation_index::ConversationRecord;
use demi_backend_database::accounts::TokenHash;
use demi_backend_database::integrations::{IntegrationPatch as StoredPatch, IntegrationRecord};
use std::sync::atomic::Ordering;

use demi_backend_user_shard::integrations::{IntegrationRefusal, IntegrationSocket};
use demi_integration_api_protocol as api;
use demi_web_api_protocol::error::ErrorCode;
use demi_conversation_lane_protocol::ClientFrame;
use demi_web_api_protocol::conversations::{TranscriptQuery, WholeBlock};
use demi_web_api_protocol::files::{
    CreateDirectory, CreatedDirectory, DirectoryQuery, RawFileQuery, RemoveQuery, SearchRequest,
    UploadQuery as FileUploadQuery,
};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId};
use demi_web_api_protocol::query::StrictBool;
use demi_shared_types::{BlockId, CommandId, NodeId, TurnId};

use super::AppState;
use super::view_links;
use super::body::{JsonBody, OptionalJsonBody};
use super::error::ApiError;
use super::files::{self, FileHost};
use super::listener::Peer;
use super::query::{CheckedQuery, QueryParams};

/// The routes of the integration API, under its prefix.
pub(super) fn routes() -> Router<AppState> {
    Router::new().nest(
        api::PREFIX,
        Router::new()
            .route("/openapi.json", get(openapi))
            .route("/integration", get(integration).patch(patch_integration))
            .route("/models", get(models))
            .route("/clouds", get(clouds).post(create_cloud))
            .route("/clouds/{cloud}", get(cloud).patch(patch_cloud).delete(delete_cloud))
            .route("/clouds/{cloud}/reset", post(reset_cloud))
            .route("/clouds/{cloud}/commands", post(start_cloud_command))
            .route(
                "/clouds/{cloud}/fs",
                get(list_cloud_files).post(make_cloud_directory).delete(remove_cloud_file),
            )
            .route("/clouds/{cloud}/fs/raw", get(read_cloud_file).put(write_cloud_file))
            .route("/clouds/{cloud}/fs/search", post(search_cloud_files))
            .route("/clouds/{cloud}/commands/{command}", get(cloud_command))
            .route("/clouds/{cloud}/commands/{command}/stop", post(stop_cloud_command))
            .route("/conversations", get(conversations).post(create_conversation))
            .route(
                "/conversations/{conversation}",
                get(conversation).patch(patch_conversation).delete(delete_conversation),
            )
            .route("/conversations/{conversation}/messages", post(send_message).delete(clear_queue))
            .route("/conversations/{conversation}/messages/{message}", delete(withdraw_message))
            .route("/conversations/{conversation}/messages/{message}/send-now", post(send_now))
            .route("/conversations/{conversation}/stop", post(stop))
            .route("/conversations/{conversation}/resume", post(resume))
            .route("/conversations/{conversation}/retry", post(retry))
            .route("/conversations/{conversation}/compact", post(compact))
            .route("/conversations/{conversation}/fork", post(fork))
            .route("/conversations/{conversation}/commands/{command}/stop", post(stop_command))
            .route("/conversations/{conversation}/agents/{agent}/stop", post(stop_agent))
            .route("/conversations/{conversation}/agents/{agent}/compact", post(compact_agent))
            .route("/conversations/{conversation}/agents/{agent}/messages", post(message_agent))
            .route("/conversations/{conversation}/agents/{agent}/transcript", get(transcript))
            .route("/conversations/{conversation}/agents/{agent}/blocks/{block}", get(block))
            .route("/conversations/{conversation}/commands/{command}/output", get(command_output))
            .route(
                "/conversations/{conversation}/fs",
                get(list_files).post(make_directory).delete(remove_file),
            )
            .route("/conversations/{conversation}/fs/raw", get(read_file).put(write_file))
            .route("/conversations/{conversation}/fs/search", post(search_files))
            .route("/conversations/{conversation}/browser/tabs/{tab}/views", post(view_links::create))
            .route("/conversations/{conversation}/browser/views", get(view_links::list))
            .route("/conversations/{conversation}/browser/views/{view}", delete(view_links::remove))
            .route(
                "/conversations/{conversation}/views",
                post(view_links::create_conversation).get(view_links::list_conversation),
            )
            .route("/conversations/{conversation}/views/{view}", delete(view_links::remove_conversation))
            .route(
                "/conversations/{conversation}/hosts/{device}/fs",
                get(list_files_on).post(make_directory_on),
            )
            .route("/socket", get(socket))
            .route(
                "/attachments",
                post(upload).layer(axum::extract::DefaultBodyLimit::max(super::attachments::BODY_LIMIT)),
            )
            .route("/blobs/{sha256}", get(blob))
            .route("/conversations/{conversation}/attachments/{attachment}", get(conversation_attachment))
            .route("/usage", get(usage)),
    )
}

/// The integration whose token the request carries as its bearer token.
pub(super) async fn authenticated(state: &AppState, headers: &HeaderMap) -> Result<IntegrationRecord, ApiError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(unauthenticated)?;
    state
        .services
        .control
        .integration_by_token(TokenHash::of(token.trim()))
        .await?
        .ok_or_else(unauthenticated)
}

fn unauthenticated() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        ErrorCode::Unauthenticated,
        "The request carries no integration's token",
    )
}

pub(crate) fn refused(refusal: IntegrationRefusal) -> ApiError {
    match refusal {
        IntegrationRefusal::NotFound(what) => {
            ApiError::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, format!("The integration has no {what} of that id"))
        }
        IntegrationRefusal::Refused(message) => ApiError::new(StatusCode::CONFLICT, ErrorCode::IntegrationRefused, message),
        IntegrationRefusal::Invalid(message) => ApiError::new(StatusCode::BAD_REQUEST, ErrorCode::IntegrationRefused, message),
        refusal @ IntegrationRefusal::CloudDiffers => {
            ApiError::new(StatusCode::CONFLICT, ErrorCode::CloudDiffers, refusal.to_string())
        }
        IntegrationRefusal::Cloud(error) => error.into(),
        IntegrationRefusal::Storage(error) => error.into(),
        IntegrationRefusal::ConversationStore(error) => ApiError::internal(&error),
        IntegrationRefusal::Internal(message) => ApiError::internal_message(message),
    }
}

fn integration_answer(record: IntegrationRecord) -> api::Integration {
    api::Integration {
        id: record.id,
        name: record.name,
        capabilities: record.capabilities,
        init_script: record.init_script,
        shared_path: record.shared_path,
        cloud_sizing: record.cloud_sizing,
        cloud_idle_minutes: record.cloud_idle_minutes,
    }
}

/// `GET /v1/openapi.json`: the document of these routes, which asks for no
/// token.
async fn openapi(State(state): State<AppState>) -> Json<serde_json::Value> {
    let server = state
        .services
        .public_url
        .get()
        .map(|url| url.as_str().trim_end_matches('/').to_owned())
        .unwrap_or_default();
    Json(api::openapi(&server))
}

/// `GET /v1/integration`.
async fn integration(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<api::Integration>, ApiError> {
    Ok(Json(integration_answer(authenticated(&state, &headers).await?)))
}

/// `PATCH /v1/integration`: the capabilities.
async fn patch_integration(
    State(state): State<AppState>,
    headers: HeaderMap,
    JsonBody(patch): JsonBody<api::IntegrationPatch>,
) -> Result<Json<api::Integration>, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let stored = StoredPatch {
        name: None,
        capabilities: patch.capabilities,
        init_script: patch.init_script,
        shared_path: patch.shared_path,
        cloud_sizing: None,
        cloud_idle_minutes: patch.cloud_idle_minutes,
    };
    let record = state
        .shards
        .of(&record.user)
        .call(move |shard, _| async move { shard.change_integration(record.id, stored).await })
        .await?
        .map_err(refused)?
        .ok_or_else(unauthenticated)?;
    Ok(Json(integration_answer(record)))
}

/// `GET /v1/models`: the models the integration's conversations may run on.
async fn models(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<api::ModelList>, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let catalog = state
        .services
        .model_catalog(&record.user, false)
        .await
        .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::IntegrationRefused, error.to_string()))?;
    let models = catalog
        .providers
        .into_iter()
        .flat_map(|provider| {
            let entry = provider.provider_id.to_string();
            let name = provider.display_name.clone();
            provider.models.into_iter().map(move |model| {
                let model = model.model;
                api::Model {
                    provider_id: entry.clone(),
                    provider: name.clone(),
                    model_id: model.id,
                    name: model.display_name,
                    context_window: model.context_window,
                    output_limit: model.output_limit,
                    efforts: model.supported_thinking_efforts.unwrap_or_default(),
                    can_disable_thinking: model.can_disable_thinking.unwrap_or(false),
                    service_tiers: model.service_tiers.into_iter().map(|tier| tier.id).collect(),
                    file_types: model.accepted_extensions,
                }
            })
        })
        .collect();
    Ok(Json(api::ModelList { models }))
}

/// `GET /v1/clouds`.
async fn clouds(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<api::CloudList>, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let clouds = state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.integration_clouds(&record).await })
        .await??;
    Ok(Json(api::CloudList { clouds }))
}

/// `POST /v1/clouds`: answers 201 with the Cloud, or 200 with the one of its
/// key.
async fn create_cloud(
    State(state): State<AppState>,
    headers: HeaderMap,
    JsonBody(request): JsonBody<api::CreateCloud>,
) -> Result<(StatusCode, Json<api::Cloud>), ApiError> {
    let record = authenticated(&state, &headers).await?;
    let cloud = state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.create_integration_cloud(&record, request).await })
        .await?
        .map_err(refused)?;
    Ok((StatusCode::CREATED, Json(cloud)))
}

/// `GET /v1/clouds/:cloud`.
async fn cloud(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cloud): Path<String>,
) -> Result<Json<api::Cloud>, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let device = device_id(cloud)?;
    state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.integration_cloud(&record, &device).await })
        .await??
        .map(Json)
        .ok_or_else(|| refused(IntegrationRefusal::NotFound("Cloud")))
}

/// `POST /v1/clouds/:cloud/reset`.
async fn reset_cloud(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cloud): Path<String>,
) -> Result<Json<api::Cloud>, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let device = device_id(cloud)?;
    let cloud = state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.reset_integration_cloud(&record, &device).await })
        .await?
        .map_err(refused)?;
    Ok(Json(cloud))
}

/// `PATCH /v1/clouds/:cloud`: makes it always on, or not.
async fn patch_cloud(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cloud): Path<String>,
    JsonBody(patch): JsonBody<api::CloudPatch>,
) -> Result<Json<api::Cloud>, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let device = device_id(cloud)?;
    let cloud = state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.patch_integration_cloud(&record, &device, patch).await })
        .await?
        .map_err(refused)?;
    Ok(Json(cloud))
}

/// `POST /v1/clouds/:cloud/commands`: a script run on the Cloud outside any
/// conversation, answered at once with its start.
async fn start_cloud_command(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cloud): Path<String>,
    JsonBody(start): JsonBody<api::StartCloudCommand>,
) -> Result<(StatusCode, Json<api::CloudCommand>), ApiError> {
    let record = authenticated(&state, &headers).await?;
    let device = device_id(cloud)?;
    let command = state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.start_cloud_command(&record, &device, start).await })
        .await?
        .map_err(refused)?;
    Ok((StatusCode::CREATED, Json(command)))
}

/// `GET /v1/clouds/:cloud/commands/:command?after=`: the command, with its
/// output from a byte on.
async fn cloud_command(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((cloud, command)): Path<(String, String)>,
    CheckedQuery(query): CheckedQuery<api::OutputQuery>,
) -> Result<Json<api::CloudCommand>, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let device = device_id(cloud)?;
    let answer = state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.cloud_command(&record, &device, &command, query.after).await })
        .await?
        .map_err(refused)?;
    Ok(Json(answer))
}

/// `POST /v1/clouds/:cloud/commands/:command/stop`.
async fn stop_cloud_command(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((cloud, command)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let device = device_id(cloud)?;
    state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.stop_cloud_command(&record, &device, &command).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /v1/clouds/:cloud`: the Cloud, once no conversation works on it.
async fn delete_cloud(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cloud): Path<String>,
) -> Result<StatusCode, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let device = device_id(cloud)?;
    state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.delete_integration_cloud(&record, &device).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/attachments?name=`: a file's bytes, at most 25 MiB, into its
/// user's blobs, as the composer uploads one.
async fn upload(
    State(state): State<AppState>,
    CheckedQuery(query): CheckedQuery<api::AttachmentQuery>,
    request: axum::extract::Request,
) -> Result<(StatusCode, Json<api::Attachment>), ApiError> {
    let record = authenticated(&state, request.headers()).await?;
    garde::Validate::validate(&query).map_err(|report| ApiError::invalid_query(format!("name: {report}")))?;
    let sent = super::attachments::sent_media_type(request.headers())?;
    let too_large = || {
        ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            ErrorCode::TooLarge,
            format!("An upload is at most {} bytes", super::attachments::BODY_LIMIT),
        )
    };
    let bytes = super::body::read_within(request, &state, super::attachments::BODY_LIMIT, too_large).await?;
    if bytes.is_empty() {
        return Err(ApiError::invalid_body("An upload holds at least one byte"));
    }
    let stored = super::attachments::store(&state.services, &record.user, &query.name, &sent, bytes).await?;
    Ok((
        StatusCode::CREATED,
        Json(api::Attachment {
            id: stored.id.to_string(),
            media_type: stored.media_type,
            size_bytes: stored.size_bytes,
            sha256: stored.sha256.to_string(),
        }),
    ))
}

/// `GET /v1/blobs/:sha256`: a blob of the integration's user's, such as a
/// file an agent attached to its reply, as the web app reads one.
async fn blob(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let no_such_blob = || ApiError::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, "No blob of that SHA-256");
    let blob = demi_shared_types::BlobRef::try_from(name).map_err(|_| no_such_blob())?;
    let bytes = state
        .services
        .blobs
        .for_user(&record.user)
        .get(&blob)
        .await?
        .ok_or_else(no_such_blob)?;
    Ok(super::blobs::blob_answer(&bytes, None, &headers))
}

/// `GET /v1/conversations/:conversation/attachments/:attachment`: a file the
/// conversation holds, by its number, as a reply's `<attachment id="a3" />`
/// names it.
async fn conversation_attachment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation, attachment)): Path<(String, String)>,
) -> Result<Json<api::ConversationAttachment>, ApiError> {
    use demi_backend_database::conversation_attachments::{self, AttachmentNumber};
    let record = authenticated(&state, &headers).await?;
    let id = conversation_id(conversation)?;
    let no_file = || ApiError::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, "The conversation has no attachment of that number");
    let number: AttachmentNumber = attachment.parse().map_err(|_| no_file())?;
    let id = state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.integration_conversation_of(&record, &id).await })
        .await?
        .map_err(refused)?
        .id;
    let row = state
        .services
        .conversations
        .read(&id, move |connection| conversation_attachments::read(connection, number))
        .await
        .map_err(ApiError::conversation_unreadable)?
        .flatten()
        .ok_or_else(no_file)?;
    Ok(Json(api::ConversationAttachment {
        id: row.number.to_string(),
        name: row.name,
        media_type: row.media_type,
        size_bytes: row.size,
        sha256: row.blob.to_string(),
    }))
}

/// `GET /v1/usage`: the integration's usage in a period, grouped.
async fn usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    CheckedQuery(query): CheckedQuery<api::UsageQuery>,
) -> Result<Json<api::Usage>, ApiError> {
    use demi_backend_database::usage::UsageKey;
    let record = authenticated(&state, &headers).await?;
    let key = match query.by {
        api::UsageGrouping::Cloud => UsageKey::Device,
        api::UsageGrouping::Conversation => UsageKey::Conversation,
        api::UsageGrouping::Model => UsageKey::Model,
        api::UsageGrouping::Turn => UsageKey::Turn,
        api::UsageGrouping::Message => UsageKey::Message,
    };
    let rows = state
        .services
        .control
        .integration_usage(record.id, query.from, query.to, key)
        .await?
        .into_iter()
        .map(|row| api::UsageRow {
            key: row.key,
            requests: row.requests,
            input_tokens: row.input_tokens,
            output_tokens: row.output_tokens,
            cache_read_tokens: row.cache_read_tokens,
            cache_write_tokens: row.cache_write_tokens,
            api_cost: row.api_cost,
        })
        .collect();
    Ok(Json(api::Usage { rows }))
}

/// `GET /v1/conversations`: a page, newest first, or the one of a key.
async fn conversations(
    State(state): State<AppState>,
    headers: HeaderMap,
    CheckedQuery(query): CheckedQuery<api::ConversationQuery>,
) -> Result<Json<api::ConversationList>, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let list = state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.integration_conversation_page(&record, query).await })
        .await?
        .map_err(refused)?;
    Ok(Json(list))
}

/// `POST /v1/conversations`.
async fn create_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    JsonBody(request): JsonBody<api::CreateConversation>,
) -> Result<(StatusCode, Json<api::Conversation>), ApiError> {
    let record = authenticated(&state, &headers).await?;
    let conversation = state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.create_integration_conversation(&record, request).await })
        .await?
        .map_err(refused)?;
    Ok((StatusCode::CREATED, Json(conversation)))
}

/// `GET /v1/conversations/:conversation`.
async fn conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation): Path<String>,
) -> Result<Json<api::ConversationDetail>, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let id = conversation_id(conversation)?;
    let detail = state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.integration_conversation_detail(&record, &id).await })
        .await?
        .map_err(refused)?;
    Ok(Json(detail))
}

/// `PATCH /v1/conversations/:conversation`.
async fn patch_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation): Path<String>,
    JsonBody(patch): JsonBody<api::ConversationPatch>,
) -> Result<Json<api::Conversation>, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let id = conversation_id(conversation)?;
    let conversation = state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.patch_integration_conversation(&record, &id, patch).await })
        .await?
        .map_err(refused)?;
    Ok(Json(conversation))
}

/// `DELETE /v1/conversations/:conversation`.
async fn delete_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation): Path<String>,
) -> Result<StatusCode, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let id = conversation_id(conversation)?;
    state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.delete_integration_conversation(&record, &id).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/conversations/:conversation/messages`.
async fn send_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation): Path<String>,
    JsonBody(message): JsonBody<api::SendMessage>,
) -> Result<Json<api::MessageAnswer>, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let id = conversation_id(conversation)?;
    let answer = state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.send_integration_message(&record, &id, message).await })
        .await?
        .map_err(refused)?;
    Ok(Json(answer))
}

/// `POST /v1/conversations/:conversation/stop`: every agent, or those the
/// body's `scope` names.
async fn stop(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation): Path<String>,
    OptionalJsonBody(api::StopRequest { scope }): OptionalJsonBody<api::StopRequest>,
) -> Result<StatusCode, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let id = conversation_id(conversation)?;
    state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.stop_integration_conversation(&record, &id, scope).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/conversations/:conversation/commands/:command/output`: what an
/// agent's command printed, from `after`.
async fn command_output(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation, command)): Path<(String, String)>,
    CheckedQuery(query): CheckedQuery<api::OutputQuery>,
) -> Result<Json<api::CommandOutput>, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let id = conversation_id(conversation)?;
    let answer = state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move {
            shard.integration_command_output(&record, &id, &command, query.after).await
        })
        .await?
        .map_err(refused)?;
    Ok(Json(answer))
}

/// Hands the conversation the web app's frame `frame` makes of its id, as
/// the control of a page does: 204 once it took it.
async fn act(
    state: &AppState,
    headers: &HeaderMap,
    conversation: String,
    frame: ClientFrame,
) -> Result<StatusCode, ApiError> {
    let record = authenticated(state, headers).await?;
    let id = conversation_id(conversation)?;
    state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.act_on_integration_conversation(&record, &id, frame).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/conversations/:conversation/agents/:agent/stop`: one subagent.
async fn stop_agent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation, agent)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    act(&state, &headers, conversation, ClientFrame::AbortSubagent { subagent_id: node_id(agent)? }).await
}

/// `POST /v1/conversations/:conversation/resume`.
async fn resume(State(state): State<AppState>, headers: HeaderMap, Path(conversation): Path<String>) -> Result<StatusCode, ApiError> {
    act(&state, &headers, conversation, ClientFrame::Resume {}).await
}

/// `POST /v1/conversations/:conversation/retry`.
async fn retry(State(state): State<AppState>, headers: HeaderMap, Path(conversation): Path<String>) -> Result<StatusCode, ApiError> {
    act(&state, &headers, conversation, ClientFrame::Retry {}).await
}

/// `POST /v1/conversations/:conversation/compact`: the main agent's context.
async fn compact(State(state): State<AppState>, headers: HeaderMap, Path(conversation): Path<String>) -> Result<StatusCode, ApiError> {
    act(&state, &headers, conversation, ClientFrame::Compact { node: None }).await
}

/// `POST /v1/conversations/:conversation/agents/:agent/compact`.
async fn compact_agent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation, agent)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    act(&state, &headers, conversation, ClientFrame::Compact { node: Some(node_id(agent)?) }).await
}

/// `DELETE /v1/conversations/:conversation/messages`: the whole queue.
async fn clear_queue(State(state): State<AppState>, headers: HeaderMap, Path(conversation): Path<String>) -> Result<StatusCode, ApiError> {
    act(&state, &headers, conversation, ClientFrame::ClearMessageQueue {}).await
}

/// `DELETE /v1/conversations/:conversation/messages/:message`: a queued
/// message, or a steer the turn has not taken yet.
async fn withdraw_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation, message)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    act_on_message(
        &state,
        &headers,
        (conversation, message),
        |message_id| ClientFrame::DequeueMessage { message_id },
        |steer_id| ClientFrame::CancelPendingSteer { steer_id },
    )
    .await
}

/// `POST /v1/conversations/:conversation/messages/:message/send-now`: a
/// queued message into the running turn, or a pending steer at once.
async fn send_now(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation, message)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    act_on_message(
        &state,
        &headers,
        (conversation, message),
        |message_id| ClientFrame::SendQueuedMessage { message_id },
        |steer_id| ClientFrame::SteerNow { steer_id },
    )
    .await
}

/// Hands a waiting message of the conversation the frame `queued` or
/// `steered` makes, as it waits queued or as a steer: 204 once it took it.
async fn act_on_message(
    state: &AppState,
    headers: &HeaderMap,
    (conversation, message): (String, String),
    queued: fn(TurnId) -> ClientFrame,
    steered: fn(BlockId) -> ClientFrame,
) -> Result<StatusCode, ApiError> {
    let record = authenticated(state, headers).await?;
    let id = conversation_id(conversation)?;
    state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move {
            shard.act_on_integration_message(&record, &id, message, queued, steered).await
        })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/conversations/:conversation/commands/:command/stop`.
async fn stop_command(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation, command)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let command_id = CommandId::try_from(command).map_err(|_| refused(IntegrationRefusal::NotFound("command")))?;
    act(&state, &headers, conversation, ClientFrame::ShellAbort { command_id }).await
}

/// `POST /v1/conversations/:conversation/agents/:agent/messages`: a message
/// to a subagent, as its user's.
async fn message_agent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation, agent)): Path<(String, String)>,
    JsonBody(message): JsonBody<api::SubagentMessage>,
) -> Result<StatusCode, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let id = conversation_id(conversation)?;
    let agent = node_id(agent)?;
    state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.message_integration_subagent(&record, &id, &agent, message).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/conversations/:conversation/fork`: answers 201 with the new
/// conversation, or 200 with the one of the body's key.
async fn fork(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation): Path<String>,
    JsonBody(request): JsonBody<api::ForkRequest>,
) -> Result<(StatusCode, Json<api::Conversation>), ApiError> {
    let record = authenticated(&state, &headers).await?;
    let id = conversation_id(conversation)?;
    let (forked, created) = state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.fork_integration_conversation(&record, &id, request).await })
        .await?
        .map_err(refused)?;
    let status = if created { StatusCode::CREATED } else { StatusCode::OK };
    Ok((status, Json(forked)))
}

/// The record of the integration's conversation `conversation`, whose
/// history and Host the web app's routes read for it.
pub(super) async fn owned_conversation(
    state: &AppState,
    record: IntegrationRecord,
    conversation: String,
) -> Result<ConversationRecord, ApiError> {
    let id = conversation_id(conversation)?;
    let id = state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.integration_conversation_of(&record, &id).await })
        .await?
        .map_err(refused)?
        .id;
    state
        .services
        .control
        .conversation(id)
        .await?
        .ok_or_else(|| refused(IntegrationRefusal::NotFound("conversation")))
}

/// `GET /v1/conversations/:conversation/agents/:agent/transcript`: a page
/// of the agent's transcript, as the web app reads it
/// (`crates/backend-http/src/history.rs`), the latest without a query.
async fn transcript(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation, agent)): Path<(String, String)>,
    CheckedQuery(query): CheckedQuery<api::TranscriptQuery>,
) -> Result<Json<api::Transcript>, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let node = node_id(agent)?;
    let owner = record.user.clone();
    let conversation = owned_conversation(&state, record, conversation).await?;
    let api::TranscriptQuery { before, after, around, edge } = query;
    let latest = before.is_none() && after.is_none() && around.is_none();
    let query = TranscriptQuery {
        latest: StrictBool(latest),
        blocks: None,
        before,
        after,
        around,
        edge,
    };
    let page = super::history::read_transcript(&state, &owner, conversation, node, query).await?;
    let end = page.start + page.blocks.len() as u32;
    Ok(Json(api::Transcript {
        version: page.version,
        start: page.start,
        at_start: page.start == 0,
        at_end: end == page.length,
        blocks: page.blocks,
    }))
}

/// `GET /v1/conversations/:conversation/agents/:agent/blocks/:block`: a
/// block whole, as a row the page opens.
async fn block(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation, agent, block)): Path<(String, String, String)>,
) -> Result<Json<WholeBlock>, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let node = node_id(agent)?;
    let block = BlockId::try_from(block).map_err(|_| refused(IntegrationRefusal::NotFound("block")))?;
    let conversation = owned_conversation(&state, record, conversation).await?;
    super::history::whole_block(&state, &conversation, node, block).await.map(Json)
}

/// The id of the integration's conversation `conversation` and its user,
/// once the conversation is the integration's: what the web app's file
/// routes take (`crates/backend-http/src/files.rs`).
async fn files_of(state: &AppState, headers: &HeaderMap, conversation: String) -> Result<(UserId, String), ApiError> {
    let record = authenticated(state, headers).await?;
    let user = record.user.clone();
    let conversation = owned_conversation(state, record, conversation).await?;
    Ok((user, conversation.id.to_string()))
}

/// `GET /v1/conversations/:conversation/fs`: a directory's entries on the
/// conversation's Host.
async fn list_files(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation): Path<String>,
    QueryParams(query): QueryParams<DirectoryQuery>,
) -> Result<Response, ApiError> {
    let (user, id) = files_of(&state, &headers, conversation).await?;
    files::list_on(&state, &user, FileHost::conversation(&id, None), query).await
}

/// `GET /v1/conversations/:conversation/hosts/:device/fs`: a directory's
/// entries on another Host the conversation reaches.
async fn list_files_on(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation, device)): Path<(String, String)>,
    QueryParams(query): QueryParams<DirectoryQuery>,
) -> Result<Response, ApiError> {
    let (user, id) = files_of(&state, &headers, conversation).await?;
    files::list_on(&state, &user, FileHost::conversation(&id, Some(&device)), query).await
}

/// `POST /v1/conversations/:conversation/fs`: a directory, with its parents.
async fn make_directory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation): Path<String>,
    JsonBody(body): JsonBody<CreateDirectory>,
) -> Result<(StatusCode, Json<CreatedDirectory>), ApiError> {
    let (user, id) = files_of(&state, &headers, conversation).await?;
    files::make_directory_on(&state, &user, FileHost::conversation(&id, None), body).await
}

/// `POST /v1/conversations/:conversation/hosts/:device/fs`.
async fn make_directory_on(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation, device)): Path<(String, String)>,
    JsonBody(body): JsonBody<CreateDirectory>,
) -> Result<(StatusCode, Json<CreatedDirectory>), ApiError> {
    let (user, id) = files_of(&state, &headers, conversation).await?;
    files::make_directory_on(&state, &user, FileHost::conversation(&id, Some(&device)), body).await
}

/// `DELETE /v1/conversations/:conversation/fs`: a file, or a directory with
/// what it holds.
async fn remove_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation): Path<String>,
    QueryParams(query): QueryParams<RemoveQuery>,
) -> Result<StatusCode, ApiError> {
    let (user, id) = files_of(&state, &headers, conversation).await?;
    files::remove_of(&state, &user, FileHost::conversation(&id, None), query).await
}

/// `GET /v1/conversations/:conversation/fs/raw`: a file's bytes.
async fn read_file(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    method: Method,
    headers: HeaderMap,
    Path(conversation): Path<String>,
    QueryParams(query): QueryParams<RawFileQuery>,
) -> Result<Response, ApiError> {
    let (user, id) = files_of(&state, &headers, conversation).await?;
    files::raw_of(&state, &user, &peer, method, &headers, FileHost::conversation(&id, None), query).await
}

/// `PUT /v1/conversations/:conversation/fs/raw`: a file's bytes, written
/// whole or not at all.
async fn write_file(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path(conversation): Path<String>,
    QueryParams(query): QueryParams<FileUploadQuery>,
    body: Body,
) -> Result<StatusCode, ApiError> {
    let (user, id) = files_of(&state, &headers, conversation).await?;
    files::upload_of(&state, &user, &peer, FileHost::conversation(&id, None), query, body).await
}

/// `POST /v1/conversations/:conversation/fs/search`: the names and text
/// that match, one JSON object per line.
async fn search_files(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path(conversation): Path<String>,
    JsonBody(request): JsonBody<SearchRequest>,
) -> Result<Response, ApiError> {
    let (user, id) = files_of(&state, &headers, conversation).await?;
    files::search_of(&state, &user, &peer, FileHost::conversation(&id, None), request).await
}

/// The user and the Cloud of a Cloud's file route.
async fn cloud_files_of(state: &AppState, headers: &HeaderMap, cloud: String) -> Result<(UserId, FileHost<'static>), ApiError> {
    let record = authenticated(state, headers).await?;
    let cloud = device_id(cloud)?;
    Ok((record.user.clone(), FileHost::Cloud { integration: record, cloud }))
}

/// `GET /v1/clouds/:cloud/fs`: a directory's entries on the Cloud, its home
/// when the query names none.
async fn list_cloud_files(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cloud): Path<String>,
    QueryParams(query): QueryParams<DirectoryQuery>,
) -> Result<Response, ApiError> {
    let (user, file_host) = cloud_files_of(&state, &headers, cloud).await?;
    files::list_on(&state, &user, file_host, query).await
}

/// `POST /v1/clouds/:cloud/fs`: a directory, with its parents.
async fn make_cloud_directory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cloud): Path<String>,
    JsonBody(body): JsonBody<CreateDirectory>,
) -> Result<(StatusCode, Json<CreatedDirectory>), ApiError> {
    let (user, file_host) = cloud_files_of(&state, &headers, cloud).await?;
    files::make_directory_on(&state, &user, file_host, body).await
}

/// `DELETE /v1/clouds/:cloud/fs`: a file, or a directory with what it holds.
async fn remove_cloud_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cloud): Path<String>,
    QueryParams(query): QueryParams<RemoveQuery>,
) -> Result<StatusCode, ApiError> {
    let (user, file_host) = cloud_files_of(&state, &headers, cloud).await?;
    files::remove_of(&state, &user, file_host, query).await
}

/// `GET /v1/clouds/:cloud/fs/raw`: a file's bytes.
async fn read_cloud_file(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    method: Method,
    headers: HeaderMap,
    Path(cloud): Path<String>,
    QueryParams(query): QueryParams<RawFileQuery>,
) -> Result<Response, ApiError> {
    let (user, file_host) = cloud_files_of(&state, &headers, cloud).await?;
    files::raw_of(&state, &user, &peer, method, &headers, file_host, query).await
}

/// `PUT /v1/clouds/:cloud/fs/raw`: a file's bytes, written whole or not at
/// all.
async fn write_cloud_file(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path(cloud): Path<String>,
    QueryParams(query): QueryParams<FileUploadQuery>,
    body: Body,
) -> Result<StatusCode, ApiError> {
    let (user, file_host) = cloud_files_of(&state, &headers, cloud).await?;
    files::upload_of(&state, &user, &peer, file_host, query, body).await
}

/// `POST /v1/clouds/:cloud/fs/search`: the names and text that match, one
/// JSON object per line.
async fn search_cloud_files(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path(cloud): Path<String>,
    JsonBody(request): JsonBody<SearchRequest>,
) -> Result<Response, ApiError> {
    let (user, file_host) = cloud_files_of(&state, &headers, cloud).await?;
    files::search_of(&state, &user, &peer, file_host, request).await
}

/// `GET /v1/socket`: every change of the integration's conversations, as
/// JSON text messages, one event each; and the integration's plugin, which a
/// `declare` makes this socket serve (`crates/backend-user-shard/src/integration_plugins.rs`).
async fn socket(State(state): State<AppState>, headers: HeaderMap, upgrade: WebSocketUpgrade) -> Result<Response, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let integration = record.id.clone();
    let subscribed = state
        .shards
        .of(&record.user)
        .call(move |shard, _| async move { shard.subscribe_integration(integration) })
        .await?;
    Ok(upgrade.on_upgrade(move |socket| serve(socket, state, record, subscribed)))
}

/// Writes the socket's plugin requests first, reads what the program sends
/// before it writes events, and after the event the program reads next
/// once events were dropped, writes `behind`.
async fn serve(mut socket: WebSocket, state: AppState, record: IntegrationRecord, subscribed: IntegrationSocket) {
    let IntegrationSocket {
        id,
        mut events,
        mut control,
        behind,
    } = subscribed;
    loop {
        tokio::select! {
            biased;
            request = control.recv() => {
                let Some(request) = request else { break };
                let Ok(text) = serde_json::to_string(&request) else { continue };
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            // Before the events, which a busy integration never runs out of.
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    let message = match serde_json::from_str::<api::ClientMessage>(text.as_str()) {
                        Ok(message) => message,
                        Err(error) => {
                            let refused = api::Event::Declared { refused: Some(format!("the message is no client message: {error}")) };
                            let Ok(text) = serde_json::to_string(&refused) else { continue };
                            if socket.send(Message::Text(text.into())).await.is_err() {
                                break;
                            }
                            continue;
                        }
                    };
                    let integration = record.clone();
                    // Handled on the shard, which answers on this socket.
                    let _ = state
                        .shards
                        .of(&record.user)
                        .call(move |shard, _| async move { shard.integration_message(&integration, id, message).await })
                        .await;
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            event = events.recv() => {
                let Some(event) = event else { break };
                let Ok(text) = serde_json::to_string(&event) else { continue };
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
                // Told as the program reads again, not once it read up: one
                // that never catches up learns of the gap as well.
                if behind.swap(false, Ordering::AcqRel) {
                    let Ok(text) = serde_json::to_string(&api::Event::Behind {}) else { continue };
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
            }
        }
    }
    let integration = record.id.clone();
    let _ = state
        .shards
        .of(&record.user)
        .call(move |shard, _| async move { shard.integration_socket_closed(&integration, id) })
        .await;
}

fn conversation_id(id: String) -> Result<ConversationId, ApiError> {
    ConversationId::try_from(id.as_str()).map_err(|_| refused(IntegrationRefusal::NotFound("conversation")))
}

fn device_id(id: String) -> Result<DeviceId, ApiError> {
    DeviceId::try_from(id).map_err(|_| refused(IntegrationRefusal::NotFound("Cloud")))
}

fn node_id(id: String) -> Result<NodeId, ApiError> {
    NodeId::try_from(id).map_err(|_| refused(IntegrationRefusal::NotFound("agent")))
}

