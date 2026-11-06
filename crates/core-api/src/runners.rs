//! The routes of a runner's pipes. The mount finds the device a request is
//! from: the server by the device token it bears, a program's process by the
//! channel the runner came on.
//!
//! - `WS /api/runner/pipes` is the runner's pipe connection, whose lanes
//!   carry the stream pipes it writes (`pipe_connection`).
//! - `/api/pipes/:id` is the device ends of a pipe
//!   (`crates/runner-protocol/src/wire/pipe_frames.rs`).
//! - `WS /api/call-outputs/:id` is a call's standard output, and
//!   `/api/offers/:id` a file offered to the device.
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
use axum::extract::{ConnectInfo, FromRequestParts, Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum_extra::TypedHeader;
use axum_extra::headers::Range;
use demi_runner_client::{
    ChunkEnd, ChunkRefusal, DeviceChunk, DeviceSource, PipeRefusal, SourceMessage, accept_call_sink, offer_response,
};
use demi_runner_protocol::wire::{UPLOAD_COMPLETE, UPLOAD_OFFSET};
use demi_web_api_protocol::ids::{DeviceId, UserId};
use futures_util::StreamExt as _;

use crate::api::Api;
use crate::peer::Peer;

/// The device a runner's request is from, and the principal whose core
/// holds it.
#[derive(Debug, Clone)]
pub struct RunnerDevice {
    pub owner: UserId,
    pub id: DeviceId,
}

/// The device of a runner's request, as the mount found it: by the token
/// the request bears, or by the channel it came on.
#[derive(Debug, Clone)]
pub struct RunnerOf(pub RunnerDevice);

impl<S: Send + Sync> FromRequestParts<S> for RunnerOf {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Response> {
        parts
            .extensions
            .get::<RunnerOf>()
            .cloned()
            .ok_or_else(|| (StatusCode::UNAUTHORIZED, "device token required").into_response())
    }
}

/// The routes of a runner's pipes, for the mount to put behind what finds
/// the request's device. A WebSocket's route takes any method: over HTTP/2
/// its upgrade is a `CONNECT`.
pub fn routes() -> axum::Router<Api> {
    use axum::routing::any;
    axum::Router::new()
        .route("/api/pipes/{id}", axum::routing::put(put).get(get).head(head))
        .route(
            &format!("{}/{{id}}", demi_runner_protocol::wire::CALL_OUTPUT_ROUTE),
            any(call_output),
        )
        .route("/api/offers/{id}", axum::routing::get(offer))
        .route(
            demi_runner_protocol::wire::PIPE_CONNECTION_ROUTE,
            any(crate::pipe_connection::connect),
        )
}

/// Where a download from a byte on starts, as one that broke off resumes; a
/// range with an end, or several, is answered with the whole, as HTTP
/// allows.
pub fn range_start(range: &Range) -> u64 {
    match range.satisfiable_ranges(u64::MAX).collect::<Vec<_>>()[..] {
        [(std::ops::Bound::Included(from), std::ops::Bound::Unbounded)] => from,
        _ => 0,
    }
}

/// `PUT /api/pipes/:id`: the source's bytes, forwarded as the sink takes
/// them. The answer is the pipe's outcome: 200 once the sink drained it,
/// 409 with why it failed, also while the body is still arriving.
///
/// A request that names its offset is a chunk of an upload
/// (`crates/runner-protocol/src/wire/pipe_frames.rs`): one that is not the
/// last is answered 204 with the offset taken once its bytes went in.
pub(crate) async fn put(
    State(state): State<Api>,
    Path(id): Path<String>,
    RunnerOf(device): RunnerOf,
    headers: HeaderMap,
    body: Body,
) -> Response {
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
pub(crate) async fn get(
    State(state): State<Api>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    Path(id): Path<String>,
    RunnerOf(device): RunnerOf,
) -> Response {
    let owner = device.owner.clone();
    let claimed = state
        .shards
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
pub(crate) async fn call_output(
    State(state): State<Api>,
    Path(id): Path<String>,
    RunnerOf(device): RunnerOf,
    upgrade: WebSocketUpgrade,
) -> Response {
    let owner = device.owner.clone();
    let sink = state
        .shards
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
pub(crate) async fn offer(
    State(state): State<Api>,
    Path(id): Path<String>,
    RunnerOf(device): RunnerOf,
    range: Option<TypedHeader<Range>>,
) -> Response {
    let owner = device.owner.clone();
    let offered = state
        .shards
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
pub(crate) async fn claim_source(
    state: &Api,
    id: String,
    device: RunnerDevice,
) -> Result<DeviceSource, (StatusCode, String)> {
    let owner = device.owner.clone();
    let claimed = state
        .shards
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
pub(crate) async fn head(
    State(state): State<Api>,
    Path(id): Path<String>,
    RunnerOf(device): RunnerOf,
) -> Response {
    let owner = device.owner.clone();
    let taken = state
        .shards
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
    state: &Api,
    id: String,
    device: RunnerDevice,
    offset: u64,
) -> Result<DeviceChunk, Box<Response>> {
    let owner = device.owner.clone();
    let claimed = state
        .shards
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
