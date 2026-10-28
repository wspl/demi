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
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use axum::routing::{get, post};
use demi_backend_database::accounts::TokenHash;
use demi_backend_database::integrations::{IntegrationPatch as StoredPatch, IntegrationRecord};
use demi_backend_user_shard::integrations::IntegrationRefusal;
use demi_integration_api_protocol as api;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use demi_shared_types::NodeId;

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;

/// The routes of the integration API, under its prefix.
pub(super) fn routes() -> Router<AppState> {
    Router::new().nest(
        api::PREFIX,
        Router::new()
            .route("/openapi.json", get(openapi))
            .route("/integration", get(integration).patch(patch_integration))
            .route("/models", get(models))
            .route("/clouds", get(clouds).post(create_cloud))
            .route("/clouds/{cloud}", get(cloud))
            .route("/clouds/{cloud}/reset", post(reset_cloud))
            .route("/conversations", get(conversations).post(create_conversation))
            .route(
                "/conversations/{conversation}",
                get(conversation).patch(patch_conversation).delete(delete_conversation),
            )
            .route("/conversations/{conversation}/messages", post(send_message))
            .route("/conversations/{conversation}/stop", post(stop))
            .route("/conversations/{conversation}/agents/{agent}/transcript", get(transcript))
            .route("/socket", get(socket)),
    )
}

/// The integration whose token the request carries as its bearer token.
async fn authenticated(state: &AppState, headers: &HeaderMap) -> Result<IntegrationRecord, ApiError> {
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

fn refused(refusal: IntegrationRefusal) -> ApiError {
    match refusal {
        IntegrationRefusal::NotFound(what) => {
            ApiError::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, format!("The integration has no {what} of that id"))
        }
        IntegrationRefusal::Refused(message) => ApiError::new(StatusCode::CONFLICT, ErrorCode::IntegrationRefused, message),
        IntegrationRefusal::Storage(error) => error.into(),
    }
}

fn integration_answer(record: IntegrationRecord) -> api::Integration {
    api::Integration {
        id: record.id,
        name: record.name,
        capabilities: record.capabilities,
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
    };
    let record = state
        .services
        .control
        .update_integration(record.id, stored)
        .await?
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
            provider.models.into_iter().map(move |model| api::Model {
                provider_id: entry.clone(),
                provider: name.clone(),
                model_id: model.model.id.clone(),
                name: model.model.display_name.clone(),
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
        .call(move |shard, _| async move { shard.create_integration_cloud(&record, request.key).await })
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

/// `GET /v1/conversations`.
async fn conversations(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<api::ConversationList>, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let conversations = state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.integration_conversations(&record).await })
        .await??;
    Ok(Json(api::ConversationList { conversations }))
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

/// `POST /v1/conversations/:conversation/stop`.
async fn stop(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation): Path<String>,
) -> Result<StatusCode, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let id = conversation_id(conversation)?;
    state
        .shards
        .of(&record.user.clone())
        .call(move |shard, _| async move { shard.stop_integration_conversation(&record, &id).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/conversations/:conversation/agents/:agent/transcript`: the
/// agent's latest page, as the web app reads it
/// (`crates/backend-http/src/history.rs`).
async fn transcript(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation, agent)): Path<(String, String)>,
) -> Result<Json<api::Transcript>, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let id = conversation_id(conversation)?;
    let node = NodeId::try_from(agent).map_err(|_| refused(IntegrationRefusal::NotFound("agent")))?;
    let owner = record.user.clone();
    let id = state
        .shards
        .of(&owner)
        .call(move |shard, _| async move { shard.integration_conversation_of(&record, &id).await })
        .await?
        .map_err(refused)?
        .id;
    let conversation = state
        .services
        .control
        .conversation(id)
        .await?
        .ok_or_else(|| refused(IntegrationRefusal::NotFound("conversation")))?;
    let page = super::history::latest_page(&state, &owner, conversation, node).await?;
    Ok(Json(api::Transcript {
        version: page.version,
        blocks: page.blocks,
    }))
}

/// `GET /v1/socket`: every change of the integration's conversations, as
/// JSON text messages, one event each; and the integration's plugin, which a
/// `declare` makes this socket serve (`crates/backend-user-shard/src/integration_plugins.rs`).
async fn socket(State(state): State<AppState>, headers: HeaderMap, upgrade: WebSocketUpgrade) -> Result<Response, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let integration = record.id.clone();
    let (id, events) = state
        .shards
        .of(&record.user)
        .call(move |shard, _| async move { shard.subscribe_integration(integration) })
        .await?;
    Ok(upgrade.on_upgrade(move |socket| serve(socket, state, record, id, events)))
}

async fn serve(
    mut socket: WebSocket,
    state: AppState,
    record: IntegrationRecord,
    id: u64,
    mut events: tokio::sync::mpsc::Receiver<api::Event>,
) {
    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { break };
                let Ok(text) = serde_json::to_string(&event) else { continue };
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
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
