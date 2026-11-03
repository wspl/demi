//! An entry lent to another server (`crates/provider-relay/src/lib.rs`).
//!
//! - `/api/providers/:id/share-keys` lists, makes and revokes the entry's
//!   share keys, behind the session gate, for whoever manages the entry.
//! - `/api/relay/*` is what a borrowing server calls: no session gate and no
//!   check of the page they come from, the share key a bearer token. A
//!   request of another wire version is answered 409 with this server's,
//!   and one whose key does not lend, 401.

use std::sync::Arc;

use axum::Json;
use axum::extract::ws::{Message, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum_extra::TypedHeader;
use axum_extra::headers::Authorization;
use axum_extra::headers::authorization::Bearer;
use demi_backend_user_shard::provider_sharing::Borrowing;
use demi_backend_user_shard::services::Services;
use demi_provider_relay::wire::{ClientMessage, OtherVersion, MESSAGE_BYTES, SERVER_HEADER, VERSION, VERSION_HEADER};
use demi_web_api_protocol::providers::{NewShareKey, ShareKeys};
use futures_util::{SinkExt as _, StreamExt as _, future};

use super::AppState;
use super::error::ApiError;
use super::gate::AuthUser;
use super::providers::refused;

/// `GET /providers/:id/share-keys`.
pub(super) async fn list(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ShareKeys>, ApiError> {
    Ok(Json(services.share_keys(&user.id, &id).await.map_err(refused)?))
}

/// `POST /providers/:id/share-keys`.
pub(super) async fn create(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<NewShareKey>, ApiError> {
    Ok(Json(services.new_share_key(&user.id, &id).await.map_err(refused)?))
}

/// `DELETE /providers/:id/share-keys/:key`.
pub(super) async fn revoke(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path((id, key)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    services.revoke_share_key(&user.id, &id, &key).await.map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// What a borrowing server's request presents, checked: its wire version and
/// its key.
async fn borrowing(
    services: &Services,
    headers: &HeaderMap,
    bearer: Option<TypedHeader<Authorization<Bearer>>>,
) -> Result<Borrowing, Response> {
    let named = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    if named(VERSION_HEADER) != Some(VERSION.to_string().as_str()) {
        return Err((StatusCode::CONFLICT, Json(OtherVersion { version: VERSION })).into_response());
    }
    let refused = || StatusCode::UNAUTHORIZED.into_response();
    let Some(TypedHeader(Authorization(bearer))) = bearer else {
        return Err(refused());
    };
    // The host the borrower gives, as the key's row shows it; a name no
    // host has is shown as it came, cut to a host's length.
    let server: String = named(SERVER_HEADER).unwrap_or("").chars().take(253).collect();
    match services.borrowing(bearer.token(), server).await {
        Ok(Some(borrowing)) => Ok(borrowing),
        Ok(None) => Err(refused()),
        Err(error) => Err(ApiError::from(error).into_response()),
    }
}

/// `GET /api/relay/provider`: the lent entry's name, catalog and limits.
pub(super) async fn provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    bearer: Option<TypedHeader<Authorization<Bearer>>>,
) -> Response {
    let borrowing = match borrowing(&state.services, &headers, bearer).await {
        Ok(borrowing) => borrowing,
        Err(response) => return response,
    };
    let user = borrowing.key.created_by.clone();
    let key = borrowing.key.id.clone();
    let lent = state
        .shards
        .of(&user)
        .call(move |shard, _| async move { shard.lent(borrowing).await })
        .await;
    match lent {
        Ok(Ok(lent)) => Json(lent).into_response(),
        // The lender's own words, such as an account's state, stay here.
        Ok(Err(message)) => {
            tracing::info!(event = "relay.provider_failed", key, message, "a lent entry could not be read");
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                demi_web_api_protocol::error::ErrorCode::ProviderStatusFailed,
                "The provider could not be read.",
            )
            .into_response()
        }
        Err(unavailable) => ApiError::from(unavailable).into_response(),
    }
}

/// `WS /api/relay/socket`: one borrowed session, which the key's maker's
/// shard serves until the socket closes or the key is revoked. A frame that
/// is no message of the wire closes it.
pub(super) async fn socket(
    State(state): State<AppState>,
    headers: HeaderMap,
    bearer: Option<TypedHeader<Authorization<Bearer>>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let borrowing = match borrowing(&state.services, &headers, bearer).await {
        Ok(borrowing) => borrowing,
        Err(response) => return response,
    };
    let shards = state.shards.clone();
    upgrade
        .max_message_size(MESSAGE_BYTES)
        .max_frame_size(MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            let (sink, stream) = socket.split();
            let incoming = stream
                .scan((), |(), frame| {
                    future::ready(match frame {
                        Ok(Message::Text(text)) => serde_json::from_str::<ClientMessage>(&text).ok().map(Some),
                        Ok(Message::Ping(_) | Message::Pong(_)) => Some(None),
                        Ok(Message::Binary(_) | Message::Close(_)) | Err(_) => None,
                    })
                })
                .filter_map(future::ready);
            let outgoing = sink.with(|message| {
                let text = serde_json::to_string(&message).expect("a relay message serializes");
                future::ready(Ok::<_, axum::Error>(Message::Text(text.into())))
            });
            let user = borrowing.key.created_by.clone();
            // A closing shard takes no session: the socket drops.
            let _ = shards
                .of(&user)
                .adopt(move |shard| shard.lend(borrowing, Box::pin(incoming), Box::pin(outgoing)))
                .await;
        })
}
