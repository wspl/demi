//! An entry lent to another server (`crates/provider-relay/src/lib.rs`).
//!
//! - `/api/providers/:id/share-keys` lists, makes and revokes the entry's
//!   share keys, behind the session gate, for whoever manages the entry.
//! - `/api/relay/*` is what a borrowing server calls: no session gate and no
//!   check of the page they come from, the share key a bearer token. A
//!   request of another wire version is answered 409 with this server's,
//!   and one whose key does not lend, 401.

use std::rc::Rc;
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
use demi_backend_user_shard::shard::Shard;
use demi_provider_relay::wire::media::WireMediaFailure;
use demi_provider_relay::wire::{
    ClientMessage, OtherVersion, MESSAGE_BYTES, SERVER_HEADER, TURNED_OFF, VERSION, VERSION_HEADER,
};
use demi_shared_types::MediaKind;
use demi_web_api_protocol::providers::{NewShareKey, ShareKeys};
use futures_util::{SinkExt as _, StreamExt as _, future};
use serde::de::DeserializeOwned;

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

/// What a media call of the borrower `borrowing` answers, run by `call` on
/// the shard of its key's maker: the call's answer, or its failure with
/// HTTP 422.
async fn media_call<F, Fut, T>(state: &AppState, borrowing: Borrowing, call: F) -> Response
where
    F: FnOnce(Rc<Shard>, Borrowing) -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, WireMediaFailure>> + 'static,
    T: IntoResponse + Send + 'static,
{
    let user = borrowing.key.created_by.clone();
    match state.shards.of(&user).call(move |shard, _| call(shard, borrowing)).await {
        Ok(Ok(made)) => made.into_response(),
        Ok(Err(failure)) => (StatusCode::UNPROCESSABLE_ENTITY, Json(failure)).into_response(),
        Err(unavailable) => ApiError::from(unavailable).into_response(),
    }
}

/// The JSON body of a media call, at most [`MESSAGE_BYTES`]: an image's
/// edit and a video's frames carry their images.
async fn media_body<T: DeserializeOwned>(state: &AppState, request: Request) -> Result<T, Response> {
    let too_large = || {
        ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            demi_web_api_protocol::error::ErrorCode::TooLarge,
            format!("The request body is over its {MESSAGE_BYTES}-byte limit"),
        )
    };
    let bytes = read_within(request, state, MESSAGE_BYTES, too_large).await.map_err(IntoResponse::into_response)?;
    serde_json::from_slice(&bytes).map_err(|error| ApiError::invalid_body(error.to_string()).into_response())
}

/// `GET /api/relay/media/makers/{kind}`: the lent entry's maker of the kind.
pub(super) async fn media_maker(
    State(state): State<AppState>,
    headers: HeaderMap,
    bearer: Option<TypedHeader<Authorization<Bearer>>>,
    Path(kind): Path<MediaKind>,
) -> Response {
    let borrowing = match lending(&state.services, &headers, bearer).await {
        Ok(borrowing) => borrowing,
        Err(response) => return response,
    };
    media_call(&state, borrowing, move |shard, borrowing| async move {
        shard.lent_maker(borrowing, kind).await.map(Json)
    })
    .await
}

/// `POST /api/relay/media/image`.
pub(super) async fn media_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    bearer: Option<TypedHeader<Authorization<Bearer>>>,
    request: Request,
) -> Response {
    let borrowing = match lending(&state.services, &headers, bearer).await {
        Ok(borrowing) => borrowing,
        Err(response) => return response,
    };
    let asked = match media_body(&state, request).await {
        Ok(asked) => asked,
        Err(response) => return response,
    };
    media_call(&state, borrowing, move |shard, borrowing| async move {
        shard.lent_image(borrowing, asked).await.map(Json)
    })
    .await
}

/// `POST /api/relay/media/audio`.
pub(super) async fn media_audio(
    State(state): State<AppState>,
    headers: HeaderMap,
    bearer: Option<TypedHeader<Authorization<Bearer>>>,
    request: Request,
) -> Response {
    let borrowing = match lending(&state.services, &headers, bearer).await {
        Ok(borrowing) => borrowing,
        Err(response) => return response,
    };
    let asked = match media_body(&state, request).await {
        Ok(asked) => asked,
        Err(response) => return response,
    };
    media_call(&state, borrowing, move |shard, borrowing| async move {
        shard.lent_audio(borrowing, asked).await.map(Json)
    })
    .await
}

/// `POST /api/relay/media/audio/voices`.
pub(super) async fn media_voices(
    State(state): State<AppState>,
    headers: HeaderMap,
    bearer: Option<TypedHeader<Authorization<Bearer>>>,
    request: Request,
) -> Response {
    let borrowing = match lending(&state.services, &headers, bearer).await {
        Ok(borrowing) => borrowing,
        Err(response) => return response,
    };
    let asked = match media_body(&state, request).await {
        Ok(asked) => asked,
        Err(response) => return response,
    };
    media_call(&state, borrowing, move |shard, borrowing| async move {
        shard.lent_voices(borrowing, asked).await.map(Json)
    })
    .await
}

/// `POST /api/relay/media/video`.
pub(super) async fn media_video(
    State(state): State<AppState>,
    headers: HeaderMap,
    bearer: Option<TypedHeader<Authorization<Bearer>>>,
    request: Request,
) -> Response {
    let borrowing = match lending(&state.services, &headers, bearer).await {
        Ok(borrowing) => borrowing,
        Err(response) => return response,
    };
    let asked = match media_body(&state, request).await {
        Ok(asked) => asked,
        Err(response) => return response,
    };
    media_call(&state, borrowing, move |shard, borrowing| async move {
        shard.lent_video(borrowing, asked).await.map(Json)
    })
    .await
}

/// `GET /api/relay/media/video/{job}`: answered while the provider is
/// turned off too, as cancel is, so a job started before it was turned off
/// is not lost.
pub(super) async fn media_video_state(
    State(state): State<AppState>,
    headers: HeaderMap,
    bearer: Option<TypedHeader<Authorization<Bearer>>>,
    Path(job): Path<String>,
) -> Response {
    let borrowing = match borrowing(&state.services, &headers, bearer).await {
        Ok(borrowing) => borrowing,
        Err(response) => return response,
    };
    media_call(&state, borrowing, move |shard, borrowing| async move {
        shard.lent_video_state(borrowing, job).await.map(Json)
    })
    .await
}

/// `GET /api/relay/media/video/{job}/file`: the finished job's video, its
/// bytes, answered as its state is.
pub(super) async fn media_video_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    bearer: Option<TypedHeader<Authorization<Bearer>>>,
    Path(job): Path<String>,
) -> Response {
    let borrowing = match borrowing(&state.services, &headers, bearer).await {
        Ok(borrowing) => borrowing,
        Err(response) => return response,
    };
    media_call(&state, borrowing, move |shard, borrowing| async move {
        let video = shard.lent_video_file(borrowing, job).await?;
        Ok(([(axum::http::header::CONTENT_TYPE, "application/octet-stream")], video))
    })
    .await
}

/// `DELETE /api/relay/media/video/{job}`.
pub(super) async fn media_video_cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    bearer: Option<TypedHeader<Authorization<Bearer>>>,
    Path(job): Path<String>,
) -> Response {
    let borrowing = match borrowing(&state.services, &headers, bearer).await {
        Ok(borrowing) => borrowing,
        Err(response) => return response,
    };
    media_call(&state, borrowing, move |shard, borrowing| async move {
        shard.lent_video_cancel(&borrowing, &job).map(|()| StatusCode::NO_CONTENT)
    })
    .await
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
