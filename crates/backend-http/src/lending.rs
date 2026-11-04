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
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum_extra::TypedHeader;
use axum_extra::headers::Authorization;
use axum_extra::headers::authorization::Bearer;
use demi_backend_user_shard::provider_sharing::Borrowing;
use demi_backend_user_shard::services::Services;
use demi_backend_user_shard::shard::lent_media::Unlent;
use demi_provider_common::media::channel::Call;
use demi_provider_relay::wire::media::{Frames, KEEP_ALIVE, MAKES_NONE, frame, keep_alive};
use demi_provider_relay::wire::{
    ClientMessage, MESSAGE_BYTES, OtherVersion, SERVER_HEADER, TURNED_OFF, VERSION_HEADER, decoded, encoded, version,
};
use demi_shared_types::MediaKind;
use demi_web_api_protocol::providers::{NewShareKey, ShareKeys};
use futures_util::{SinkExt as _, StreamExt as _, future};

use super::AppState;
use super::body::read_within;
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
    if named(VERSION_HEADER) != Some(version()) {
        return Err((StatusCode::CONFLICT, Json(OtherVersion { version: version().to_owned() })).into_response());
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

/// What a borrowing server's request presents, checked as [`borrowing`]
/// checks it, for a route that lends: a provider its lender turned off is
/// answered [`TURNED_OFF`].
async fn lending(
    services: &Services,
    headers: &HeaderMap,
    bearer: Option<TypedHeader<Authorization<Bearer>>>,
) -> Result<Borrowing, Response> {
    let borrowing = borrowing(services, headers, bearer).await?;
    match services.lends(&borrowing).await {
        Ok(true) => Ok(borrowing),
        Ok(false) => Err(StatusCode::from_u16(TURNED_OFF).expect("423 is a status").into_response()),
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

/// `POST /api/relay/media/{kind}`: one call of the lent entry's maker of the
/// kind, its body one frame, answered with its replies' frames as they come
/// and an empty one every [`KEEP_ALIVE`] meanwhile
/// (`crates/provider-relay/src/wire/media.rs`). The borrower's going drops
/// the call.
pub(super) async fn media(
    State(state): State<AppState>,
    Path(kind): Path<MediaKind>,
    headers: HeaderMap,
    bearer: Option<TypedHeader<Authorization<Bearer>>>,
    request: Request,
) -> Response {
    let borrowing = match lending(&state.services, &headers, bearer).await {
        Ok(borrowing) => borrowing,
        Err(response) => return response,
    };
    let too_large = || {
        ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            demi_web_api_protocol::error::ErrorCode::TooLarge,
            format!("The request body is over its {MESSAGE_BYTES}-byte limit"),
        )
    };
    let body = match read_within(request, &state, MESSAGE_BYTES + 4, too_large).await {
        Ok(body) => body,
        Err(error) => return error.into_response(),
    };
    let mut frames = Frames::default();
    frames.push(&body);
    let call = match frames.next() {
        Ok(Some(message)) if !frames.is_partial() => decoded::<Call>(&message),
        _ => None,
    };
    let Some(call) = call else {
        return ApiError::invalid_body("The body is no call of a maker".to_owned()).into_response();
    };
    let user = borrowing.key.created_by.clone();
    let taken = state
        .shards
        .of(&user)
        .call(move |shard, _| async move { shard.lent_media(borrowing, kind, call).await })
        .await;
    let mut replies = match taken {
        Ok(Ok(replies)) => replies,
        Ok(Err(Unlent::MakesNone)) => return StatusCode::from_u16(MAKES_NONE).expect("410 is a status").into_response(),
        // As for a key refused before the call.
        Ok(Err(Unlent::Revoked)) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(unavailable) => return ApiError::from(unavailable).into_response(),
    };
    let frames = async_stream::stream! {
        let mut quiet = tokio::time::interval(KEEP_ALIVE);
        quiet.tick().await;
        loop {
            tokio::select! {
                reply = replies.recv() => match reply.as_ref().and_then(frame) {
                    Some(frame) => {
                        yield Ok::<_, std::convert::Infallible>(frame);
                        quiet.reset();
                    }
                    // The call ended, or sent what does not cross, which
                    // ends it too.
                    None => return,
                },
                _ = quiet.tick() => yield Ok(keep_alive()),
            }
        }
    };
    (
        [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
        axum::body::Body::from_stream(frames),
    )
        .into_response()
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
    let borrowing = match lending(&state.services, &headers, bearer).await {
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
                        Ok(Message::Binary(bytes)) => decoded::<ClientMessage>(&bytes).map(Some),
                        Ok(Message::Ping(_) | Message::Pong(_)) => Some(None),
                        Ok(Message::Text(_) | Message::Close(_)) | Err(_) => None,
                    })
                })
                .filter_map(future::ready);
            // A message that does not cross closes the socket.
            let outgoing = sink.with(|message| {
                future::ready(encoded(&message).map(Message::Binary).ok_or_else(|| axum::Error::new("a relay message does not cross")))
            });
            let user = borrowing.key.created_by.clone();
            // A closing shard takes no session: the socket drops.
            let _ = shards
                .of(&user)
                .adopt(move |shard| shard.lend(borrowing, Box::pin(incoming), Box::pin(outgoing)))
                .await;
        })
}
