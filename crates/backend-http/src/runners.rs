//! The routes runners use, which authenticate with a device token instead of
//! the user's session cookie (`web-api.md` § Resource index):
//! `WS /api/runner`, the one socket each runner holds, and the device ends
//! of a pipe at `/api/pipes/:id` (`runner.md` § Host operations, § Pipes and
//! output). The source's runner puts a content pipe's bytes as a request
//! body and sends a stream pipe's over a WebSocket, the sink's gets them,
//! and the edge copies them between the requests and the pipe's ends; the
//! pipe records are the device owner's shard's. Pipes have no size limit
//! and no idle timeout: a quiet command's pipe stays open for as long as its
//! command runs.

use axum::body::Body;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, UPGRADE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum_extra::TypedHeader;
use axum_extra::headers::Authorization;
use axum_extra::headers::authorization::Bearer;
use demi_backend_database::accounts::TokenHash;
use demi_backend_database::devices::DeviceRecord;
use demi_backend_remote_host::{DeviceSource, PipeRefusal};
use demi_runner_protocol::release::{RELEASE_HEADER, RunnerUpdate, TARGET_HEADER, UpdateExecutable};
use demi_runner_protocol::wire::{
    MAX_MESSAGE_BYTES, STREAM_PIPE_MESSAGE_BYTES, STREAM_PIPE_REASON_BYTES,
};
use futures_util::{SinkExt as _, StreamExt as _};

use super::AppState;
use super::error::ApiError;
use super::install::current_runner_release;
use super::runner_socket::accept;

/// `WS /api/runner`: the socket carries MessagePack frames of at most the
/// wire's limit; a larger one closes it. A backend with runner releases
/// opens it only for a runner of its current release, and answers any other
/// with 409 and the release to update to (`runner.md` § Runner updates).
pub(super) async fn socket(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    if let Some(update) = runner_update(&state, &headers).await? {
        let answer = (
            StatusCode::CONFLICT,
            [(CONTENT_TYPE, "application/json")],
            serde_json::to_vec(&update).map_err(|error| ApiError::internal_message(error.to_string()))?,
        );
        return Ok(answer.into_response());
    }
    Ok(upgrade
        .max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| accept(state.services, state.shards, socket)))
}

/// The update a runner must make before its socket opens: none from a
/// backend without runner releases or for a runner of the current release.
async fn runner_update(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<RunnerUpdate>, ApiError> {
    let Some(current) = current_runner_release(&state.site).await? else {
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
        Err(refusal) => return *refusal,
    };
    let source = match claim_source(&state, id, device).await {
        Ok(source) => source,
        Err(refusal) => return *refusal,
    };
    match source.pump(body.into_data_stream()).await {
        Ok(()) => (StatusCode::OK, "drained").into_response(),
        Err(failure) => (StatusCode::CONFLICT, failure.to_string()).into_response(),
    }
}

/// `GET /api/pipes/:id`: the pipe's bytes for its sink, or, as a WebSocket
/// upgrade, a stream pipe's bytes from its source. A sink's answer's head
/// goes out once the source is there; a pipe that fails first answers 409.
pub(super) async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
    authorization: Option<TypedHeader<Authorization<Bearer>>>,
    headers: HeaderMap,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let device = match device_of(&state, authorization).await {
        Ok(device) => device,
        Err(refusal) => return *refusal,
    };
    match upgrade {
        Ok(upgrade) => return stream_source(&state, id, device, upgrade).await,
        // An upgrade the request asks for wrongly is no sink's GET.
        Err(rejection) if headers.contains_key(UPGRADE) => return rejection.into_response(),
        Err(_) => {}
    }
    let owner = device.user.clone();
    let claimed = state
        .shards
        .of(&owner)
        .call_while_closing(move |shard, _| async move {
            shard.pipes().claim_sink(&id, device.id.as_str())
        })
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
        [
            (CONTENT_TYPE, "application/octet-stream"),
            (CACHE_CONTROL, "no-store"),
        ],
        Body::from_stream(sink.into_stream()),
    )
        .into_response()
}

/// The source end of pipe `id`, which `device` claims; the refusal answers
/// the request.
async fn claim_source(
    state: &AppState,
    id: String,
    device: DeviceRecord,
) -> Result<DeviceSource, Box<Response>> {
    let owner = device.user.clone();
    let claimed = state
        .shards
        .of(&owner)
        .call_while_closing(move |shard, _| async move {
            shard.pipes().claim_source(&id, device.id.as_str())
        })
        .await;
    match claimed {
        Ok(Ok(source)) => Ok(source),
        Ok(Err(refusal)) => Err(Box::new(refused(refusal))),
        Err(_) => Err(Box::new(StatusCode::SERVICE_UNAVAILABLE.into_response())),
    }
}

/// `WS /api/pipes/:id`: a stream pipe's source (`runner.md` § Host
/// operations). Its binary messages are the bytes, forwarded as the sink
/// takes them; the runner's normal close frame ends the pipe, and any other
/// close, or a connection lost without one, fails it. The backend's close
/// frame is the pipe's outcome, which comes early when the sink stops taking
/// bytes or the pipe fails while the runner still sends: normal once the
/// sink drained it, an error with why it failed. A refusal is an HTTP answer
/// before the upgrade.
async fn stream_source(
    state: &AppState,
    id: String,
    device: DeviceRecord,
    upgrade: WebSocketUpgrade,
) -> Response {
    let source = match claim_source(state, id, device).await {
        Ok(source) => source,
        Err(refusal) => return *refusal,
    };
    // An upgrade that never completes drops the source, which fails the pipe.
    upgrade
        .max_message_size(STREAM_PIPE_MESSAGE_BYTES)
        .max_frame_size(STREAM_PIPE_MESSAGE_BYTES)
        .on_upgrade(move |socket| relay_stream(socket, source))
}

async fn relay_stream(socket: WebSocket, source: DeviceSource) {
    let (mut to_runner, from_runner) = socket.split();
    let body = futures_util::stream::unfold(Some(from_runner), |reader| async move {
        let mut reader = reader?;
        loop {
            let failure = match reader.next().await {
                Some(Ok(Message::Binary(bytes))) => return Some((Ok(bytes), Some(reader))),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                Some(Ok(Message::Close(None))) => return None,
                Some(Ok(Message::Close(Some(frame)))) if frame.code == close_code::NORMAL => {
                    return None;
                }
                Some(Ok(Message::Close(Some(frame)))) => {
                    format!("the runner closed the stream pipe ({}): {}", frame.code, frame.reason)
                }
                Some(Ok(Message::Text(_))) => "a stream pipe carries binary messages".to_owned(),
                Some(Err(error)) => error.to_string(),
                None => "the stream pipe's connection ended without a close frame".to_owned(),
            };
            return Some((Err(failure), None));
        }
    });
    let frame = match source.pump(body).await {
        Ok(()) => CloseFrame {
            code: close_code::NORMAL,
            reason: "drained".into(),
        },
        Err(failure) => {
            let reason = failure.to_string();
            let reason = &reason[..reason.floor_char_boundary(STREAM_PIPE_REASON_BYTES)];
            CloseFrame {
                code: close_code::ERROR,
                reason: reason.into(),
            }
        }
    };
    // After the runner's own close this sends the answer to it; a runner
    // that went meanwhile hears nothing, and its pipe has ended either way.
    let _closed = to_runner.send(Message::Close(Some(frame))).await;
}

/// The device whose token the request bears; 401 without one the backend
/// issued.
async fn device_of(
    state: &AppState,
    authorization: Option<TypedHeader<Authorization<Bearer>>>,
) -> Result<DeviceRecord, Box<Response>> {
    let unauthorized =
        || Box::new((StatusCode::UNAUTHORIZED, "device token required").into_response());
    let TypedHeader(Authorization(bearer)) = authorization.ok_or_else(unauthorized)?;
    match state
        .services
        .control
        .device_by_token(TokenHash::of(bearer.token()))
        .await
    {
        Ok(Some(device)) => Ok(device),
        Ok(None) => Err(unauthorized()),
        Err(error) => {
            tracing::error!(
                error = &error as &dyn std::error::Error,
                "a pipe's device could not be read"
            );
            Err(Box::new(StatusCode::INTERNAL_SERVER_ERROR.into_response()))
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
