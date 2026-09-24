//! The routes runners use, which authenticate with a device token instead of
//! a browser session (`web-api.md` § Resource index): `WS /api/runner`, the
//! one socket each runner holds, and `PUT/GET /api/pipes/:id`, the device
//! ends of a pipe (`runner.md` § Pipes and output). The source's runner puts
//! the bytes, the sink's gets them, and the edge copies them between the
//! request bodies and the pipe's ends; the pipe records are the device
//! owner's shard's. Pipe bodies have no size limit and no idle timeout: a
//! quiet command's pipe stays open for as long as its command runs.

use axum::body::Body;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use axum_extra::TypedHeader;
use axum_extra::headers::Authorization;
use axum_extra::headers::authorization::Bearer;
use demi_host_remote::PipeRefusal;
use demi_runner_protocol::wire::MAX_MESSAGE_BYTES;

use super::AppState;
use crate::auth::sessions::TokenHash;
use crate::runner::accept::accept;
use crate::storage::devices::DeviceRecord;

/// `WS /api/runner`: the socket carries MessagePack frames of at most the
/// wire's limit; a larger one closes it.
pub(super) async fn socket(State(state): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    upgrade
        .max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| accept(state.services, state.shards, socket))
}

/// `PUT /api/pipes/:id`: the source's bytes, forwarded as the sink takes
/// them. The answer is the pipe's outcome: 200 once the sink drained it,
/// 409 with why it failed, also while the body is still arriving.
pub(super) async fn put(
    State(state): State<AppState>,
    Path(id): Path<String>,
    authorization: Option<TypedHeader<Authorization<Bearer>>>,
    body: Body,
) -> Response {
    let device = match device_of(&state, authorization).await {
        Ok(device) => device,
        Err(refusal) => return refusal,
    };
    let owner = device.user.clone();
    let claimed = state
        .shards
        .of(&owner)
        .call_while_closing(move |shard, _| async move { shard.pipes().claim_source(&id, device.id.as_str()) })
        .await;
    let source = match claimed {
        Ok(Ok(source)) => source,
        Ok(Err(refusal)) => return refused(refusal),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    match source.pump(body.into_data_stream()).await {
        Ok(()) => (StatusCode::OK, "drained").into_response(),
        Err(failure) => (StatusCode::CONFLICT, failure.to_string()).into_response(),
    }
}

/// `GET /api/pipes/:id`: the pipe's bytes for its sink. The answer's head
/// goes out once the source is there; a pipe that fails first answers 409.
pub(super) async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
    authorization: Option<TypedHeader<Authorization<Bearer>>>,
) -> Response {
    let device = match device_of(&state, authorization).await {
        Ok(device) => device,
        Err(refusal) => return refusal,
    };
    let owner = device.user.clone();
    let claimed = state
        .shards
        .of(&owner)
        .call_while_closing(move |shard, _| async move { shard.pipes().claim_sink(&id, device.id.as_str()) })
        .await;
    let mut sink = match claimed {
        Ok(Ok(sink)) => sink,
        Ok(Err(refusal)) => return refused(refusal),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    if let Err(failure) = sink.source_arrived().await {
        return (StatusCode::CONFLICT, failure.to_string()).into_response();
    }
    (
        [(CONTENT_TYPE, "application/octet-stream"), (CACHE_CONTROL, "no-store")],
        Body::from_stream(sink.into_stream()),
    )
        .into_response()
}

/// The device whose token the request bears; 401 without one the backend
/// issued.
async fn device_of(
    state: &AppState,
    authorization: Option<TypedHeader<Authorization<Bearer>>>,
) -> Result<DeviceRecord, Response> {
    let unauthorized = || (StatusCode::UNAUTHORIZED, "device token required").into_response();
    let TypedHeader(Authorization(bearer)) = authorization.ok_or_else(unauthorized)?;
    match state.services.control.device_by_token(TokenHash::of(bearer.token())).await {
        Ok(Some(device)) => Ok(device),
        Ok(None) => Err(unauthorized()),
        Err(error) => {
            tracing::error!(error = &error as &dyn std::error::Error, "a pipe's device could not be read");
            Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

fn refused(refusal: PipeRefusal) -> Response {
    let status = match refusal {
        PipeRefusal::NotFound => StatusCode::NOT_FOUND,
        PipeRefusal::AlreadyConnected => StatusCode::CONFLICT,
    };
    (status, refusal.to_string()).into_response()
}
