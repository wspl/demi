//! The routes runners use, which authenticate with a device token instead of
//! the user's session cookie.
//!
//! - `WS /api/runner` is the runner's link.
//! - `WS /api/runner/pipes` is its pipe connection, whose lanes carry the
//!   stream pipes it writes (`pipe_connection`).
//! - `/api/pipes/:id` is the device ends of a pipe
//!   (`crates/runner-protocol/src/wire/pipe_frames.rs`).
//!
//! A pipe works as follows:
//!
//! - The source's runner puts a content pipe's bytes as a request body and
//!   sends a stream pipe's over a lane of its pipe connection.
//! - The sink's runner gets them.
//! - The edge copies them between the requests and the pipe's ends.
//! - The pipe records are the device owner's core's.
//!
//! Pipes have no size limit and no idle timeout: a quiet command's pipe stays
//! open for as long as its command runs.

use axum::body::Body;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum_extra::TypedHeader;
use axum_extra::headers::{Authorization, Range};
use axum_extra::headers::authorization::Bearer;
use demi_backend_database::accounts::TokenHash;
use demi_core_store::StorageError;
use demi_web_api_protocol::ids::{DeviceId, UserId};
use demi_runner_client::{
    ChunkEnd, ChunkRefusal, DeviceChunk, DeviceSource, PipeRefusal, SourceMessage, accept_call_sink, offer_response,
};
use demi_runner_protocol::release::{
    RELEASE_HEADER, RunnerUpdate, TARGET_HEADER, TOKEN_HEADER, UpdateExecutable,
};
use demi_runner_protocol::values::DeviceToken;
use demi_runner_protocol::wire::{LINK_FRAME_BYTES, UPLOAD_COMPLETE, UPLOAD_OFFSET};
use demi_web_api_protocol::devices::DeviceKind;
use futures_util::StreamExt as _;
use tracing::Instrument as _;

use super::AppState;
use demi_core_api::error::ApiError;
use super::install::{current_runner_release, range_start};
use demi_core_api::peer::Peer;
use super::runner_socket::accept;
use demi_core_hosts::devices::RunnerSocket;

/// `WS /api/runner`: the socket carries MessagePack frames of at most the
/// wire's limit; a larger one closes it. A backend with runner releases
/// opens it only for a runner of its current release, and answers any other
/// with 409 and the release to update to (`crates/runner/src/update.rs`).
pub(super) async fn socket(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    if let Some(update) = runner_update(&state, &headers).await? {
        if update.executable.is_some() {
            updating(&state, &headers).await;
        }
        let answer = (
            StatusCode::CONFLICT,
            [(CONTENT_TYPE, "application/json")],
            serde_json::to_vec(&update).map_err(|error| ApiError::internal_message(error.to_string()))?,
        );
        return Ok(answer.into_response());
    }
    // The runner's socket logs under the request ID of its upgrade.
    let span = tracing::Span::current();
    Ok(upgrade
        .max_message_size(LINK_FRAME_BYTES)
        .max_frame_size(LINK_FRAME_BYTES)
        .on_upgrade(move |socket| {
            accept(state.services, state.api.shards, RunnerSocket::new(socket, peer.control.write_progress()))
                .instrument(span)
        }))
}

/// The update a runner must make before its socket opens: none from a
/// backend without runner releases or for a runner of the current release.
async fn runner_update(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<RunnerUpdate>, ApiError> {
    let Some(current) = current_runner_release(state).await? else {
        return Ok(None);
    };
    let named = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    if named(RELEASE_HEADER) == Some(current.release.as_str()) {
        return Ok(None);
    }
    let executable = named(TARGET_HEADER)
        .and_then(|target| current.targets.get(target))
        .map(|artifact| UpdateExecutable {
            sha256: artifact.sha256.clone(),
            size: artifact.size,
        });
    Ok(Some(RunnerUpdate {
        release: current.release,
        executable,
    }))
}

/// The device a runner's token names, and the user whose core holds it.
#[derive(Clone)]
pub(super) struct TokenDevice {
    pub(super) owner: UserId,
    pub(super) id: DeviceId,
}

/// The paired device whose token a runner's request carries in
/// `Demi-Runner-Token`: none for a request without one, from a runner of a
/// release before 0.1.14, or with one no paired device holds.
pub(super) async fn paired_device(state: &AppState, headers: &HeaderMap) -> Option<TokenDevice> {
    let token = headers
        .get(TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| DeviceToken::try_from(value.to_owned()).ok())?;
    let services = &state.services;
    let read = async {
        let Some((id, owner)) = services.control.device_by_token(TokenHash::of(token.expose())).await? else {
            return Ok(None);
        };
        let device = services.stores.of(&owner).await?.device(id).await?;
        Ok::<_, StorageError>(device.map(|device| (owner, device)))
    };
    match read.await {
        Ok(Some((owner, device))) if device.kind == DeviceKind::User => Some(TokenDevice { owner, id: device.id }),
        Ok(_) => None,
        Err(error) => {
            // What the device's runner asked for goes ahead; its device shows
            // nothing of it.
            tracing::warn!(
                event = "runner_device.read_failed",
                route = "runner_request",
                error = &error as &dyn std::error::Error,
                "the device of a runner's request could not be read"
            );
            None
        }
    }
}

/// Shows the device the request's token names as updating, since the 409
/// sends its runner to an update (`crates/runner/src/update.rs`).
async fn updating(state: &AppState, headers: &HeaderMap) {
    let Some(device) = paired_device(state, headers).await else {
        return;
    };
    let id = device.id;
    let shown = state
        .api.shards
        .of(&device.owner)
        .call(move |core, _| async move { core.runner_updating(&id) })
        .await;
    // A closing backend shows nothing more.
    let _ = shown;
}

/// `PUT /api/pipes/:id`: the source's bytes, forwarded as the sink takes
/// them. The answer is the pipe's outcome: 200 once the sink drained it,
/// 409 with why it failed, also while the body is still arriving.
///
/// A request that names its offset is a chunk of an upload
/// (`crates/runner-protocol/src/wire/pipe_frames.rs`): one that is not the
/// last is answered 204 with the offset taken once its bytes went in.
pub(super) async fn put(
    State(state): State<AppState>,
    Path(id): Path<String>,
    authorization: Option<TypedHeader<Authorization<Bearer>>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let device = match device_of(&state, authorization).await {
        Ok(device) => device,
        Err(refusal) => return *refusal,
    };
    if let Some(offset) = headers.get(UPLOAD_OFFSET) {
        let Some(offset) = offset.to_str().ok().and_then(|offset| offset.parse::<u64>().ok()) else {
            return (StatusCode::BAD_REQUEST, "the upload's offset is not a number").into_response();
        };
        let complete = headers.get(UPLOAD_COMPLETE).is_some_and(|complete| complete == "?1");
        let chunk = match claim_chunk(&state, id, device, offset).await {
            Ok(chunk) => chunk,
            Err(refusal) => return *refusal,
        };
        return match chunk.pump(body.into_data_stream(), complete).await {
            ChunkEnd::Taken(taken) => (StatusCode::NO_CONTENT, [(UPLOAD_OFFSET, taken.to_string())]).into_response(),
            // Nobody reads the answer of a request that broke off.
            ChunkEnd::Broken => StatusCode::BAD_REQUEST.into_response(),
            ChunkEnd::Over(Ok(())) => (StatusCode::OK, "drained").into_response(),
            ChunkEnd::Over(Err(failure)) => (StatusCode::CONFLICT, failure.reason().to_owned()).into_response(),
        };
    }
    let source = match claim_source(&state, id, device).await {
        Ok(source) => source,
        Err(refusal) => return refusal.into_response(),
    };
    let body = body.into_data_stream().map(|chunk| chunk.map(SourceMessage::Bytes));
    match source.pump(body).await {
        Ok(()) => (StatusCode::OK, "drained").into_response(),
        Err(failure) => (StatusCode::CONFLICT, failure.reason().to_owned()).into_response(),
    }
}

/// `GET /api/pipes/:id`: the pipe's bytes for its sink. The answer goes out
/// at once, its body in the pipe's frames (`crates/runner-protocol/src/wire/pipe_frames.rs`),
/// which say how the pipe ended.
pub(super) async fn get(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    Path(id): Path<String>,
    authorization: Option<TypedHeader<Authorization<Bearer>>>,
) -> Response {
    let device = match device_of(&state, authorization).await {
        Ok(device) => device,
        Err(refusal) => return *refusal,
    };
    let owner = device.owner.clone();
    let claimed = state
        .api.shards
        .of(&owner)
        .call_while_closing(move |core, _| async move {
            core.pipes().claim_sink(&id, device.id.as_str())
        })
        .await;
    let sink = match claimed {
        Ok(Ok(sink)) => sink,
        Ok(Err(refusal)) => return refused(refusal),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    // A write whose reply is overdue closes this connection, which ends the
    // device's read of what it still queues (`crates/runner-host/src/files.rs`).
    sink.closes_with(peer.control.closer());
    let frames = sink.into_frames().map(Ok::<_, std::convert::Infallible>);
    (
        [
            (CONTENT_TYPE, "application/octet-stream"),
            (CACHE_CONTROL, "no-store"),
        ],
        Body::from_stream(frames),
    )
        .into_response()
}

/// `WS /api/call-outputs/:id`: a call's standard output, which the runner
/// reads over a WebSocket that resumes where it got to after a drop
/// (`crates/runner-protocol/src/wire/session.rs`). A refusal is an HTTP answer
/// before the upgrade.
pub(super) async fn call_output(
    State(state): State<AppState>,
    Path(id): Path<String>,
    authorization: Option<TypedHeader<Authorization<Bearer>>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let device = match device_of(&state, authorization).await {
        Ok(device) => device,
        Err(refusal) => return *refusal,
    };
    let owner = device.owner.clone();
    let sink = state
        .api.shards
        .of(&owner)
        .call_while_closing(move |core, _| async move {
            core.devices().session(&device.id).call_output(core.pipes(), &id)
        })
        .await;
    match sink {
        Ok(Ok(sink)) => accept_call_sink(upgrade, sink),
        Ok(Err(refusal)) => refused(refusal),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// `GET /api/offers/:id`: a file the backend offers the requesting device's
/// runner for download, such as an attachment placed on its Host.
///
/// The download starts from the byte a Range request names, so a download that
/// broke off resumes (`crates/shared-artifacts/src/download.rs`). An offer
/// withdrawn, unknown or made to another device answers 404.
pub(super) async fn offer(
    State(state): State<AppState>,
    Path(id): Path<String>,
    authorization: Option<TypedHeader<Authorization<Bearer>>>,
    range: Option<TypedHeader<Range>>,
) -> Response {
    let device = match device_of(&state, authorization).await {
        Ok(device) => device,
        Err(refusal) => return *refusal,
    };
    let owner = device.owner.clone();
    let offered = state
        .api.shards
        .of(&owner)
        .call_while_closing(move |core, _| async move {
            core.pipes().offers().bytes(&id, device.id.as_str())
        })
        .await;
    match offered {
        Ok(Some(bytes)) => {
            let from = range.map_or(0, |TypedHeader(range)| range_start(&range));
            offer_response(bytes, from)
        }
        Ok(None) => (StatusCode::NOT_FOUND, "no such offer").into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// The source end of pipe `id`, which `device` claims, or the status and
/// words of its refusal.
pub(super) async fn claim_source(
    state: &AppState,
    id: String,
    device: TokenDevice,
) -> Result<DeviceSource, (StatusCode, String)> {
    let owner = device.owner.clone();
    let claimed = state
        .api.shards
        .of(&owner)
        .call_while_closing(move |core, _| async move {
            core.pipes().claim_source(&id, device.id.as_str())
        })
        .await;
    match claimed {
        Ok(Ok(source)) => Ok(source),
        Ok(Err(refusal)) => Err(refusal_of(refusal)),
        Err(_) => Err((StatusCode::SERVICE_UNAVAILABLE, String::new())),
    }
}

/// `HEAD /api/pipes/:id`: the offset of the device's upload into the pipe
/// that the backend took, from which a chunk whose connection broke goes
/// again.
pub(super) async fn head(
    State(state): State<AppState>,
    Path(id): Path<String>,
    authorization: Option<TypedHeader<Authorization<Bearer>>>,
) -> Response {
    let device = match device_of(&state, authorization).await {
        Ok(device) => device,
        Err(refusal) => return *refusal,
    };
    let owner = device.owner.clone();
    let taken = state
        .api.shards
        .of(&owner)
        .call_while_closing(move |core, _| async move {
            core.pipes().taken_by(&id, device.id.as_str())
        })
        .await;
    match taken {
        Ok(Ok(taken)) => (StatusCode::NO_CONTENT, [(UPLOAD_OFFSET, taken.to_string())]).into_response(),
        Ok(Err(refusal)) => refused(refusal),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// The source end of pipe `id` for a chunk of `device`'s upload from
/// `offset`; another offset is answered 409 with the one taken.
async fn claim_chunk(
    state: &AppState,
    id: String,
    device: TokenDevice,
    offset: u64,
) -> Result<DeviceChunk, Box<Response>> {
    let owner = device.owner.clone();
    let claimed = state
        .api.shards
        .of(&owner)
        .call_while_closing(move |core, _| async move {
            core.pipes().claim_chunk(&id, device.id.as_str(), offset)
        })
        .await;
    match claimed {
        Ok(Ok(chunk)) => Ok(chunk),
        Ok(Err(ChunkRefusal::Offset(taken))) => Err(Box::new(
            (StatusCode::CONFLICT, [(UPLOAD_OFFSET, taken.to_string())], ChunkRefusal::Offset(taken).to_string())
                .into_response(),
        )),
        Ok(Err(ChunkRefusal::NotFound)) => Err(Box::new(refused(PipeRefusal::NotFound))),
        Err(_) => Err(Box::new(StatusCode::SERVICE_UNAVAILABLE.into_response())),
    }
}

/// The device whose token the request bears; 401 without one the backend
/// issued.
pub(super) async fn device_of(
    state: &AppState,
    authorization: Option<TypedHeader<Authorization<Bearer>>>,
) -> Result<TokenDevice, Box<Response>> {
    let unauthorized =
        || Box::new((StatusCode::UNAUTHORIZED, "device token required").into_response());
    let TypedHeader(Authorization(bearer)) = authorization.ok_or_else(unauthorized)?;
    match state
        .services
        .control
        .device_by_token(TokenHash::of(bearer.token()))
        .await
    {
        Ok(Some((id, owner))) => Ok(TokenDevice { owner, id }),
        Ok(None) => Err(unauthorized()),
        Err(error) => {
            tracing::error!(
                event = "runner_device.read_failed",
                route = "pipe",
                error = &error as &dyn std::error::Error,
                "a pipe's device could not be read"
            );
            Err(Box::new(StatusCode::INTERNAL_SERVER_ERROR.into_response()))
        }
    }
}

fn refused(refusal: PipeRefusal) -> Response {
    refusal_of(refusal).into_response()
}

fn refusal_of(refusal: PipeRefusal) -> (StatusCode, String) {
    let status = match refusal {
        PipeRefusal::NotFound => StatusCode::NOT_FOUND,
        PipeRefusal::AlreadyConnected => StatusCode::CONFLICT,
    };
    (status, refusal.to_string())
}
