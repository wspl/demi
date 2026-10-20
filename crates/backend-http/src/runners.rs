//! The routes runners use, which authenticate with a device token instead of
//! the user's session cookie.
//!
//! - `WS /api/runner` is the one socket each runner holds.
//! - `/api/pipes/:id` is the device ends of a pipe
//!   (`crates/runner-protocol/src/wire/pipe_frames.rs`).
//!
//! A pipe works as follows:
//!
//! - The source's runner puts a content pipe's bytes as a request body and
//!   sends a stream pipe's over a WebSocket.
//! - The sink's runner gets them.
//! - The edge copies them between the requests and the pipe's ends.
//! - The pipe records are the device owner's shard's.
//!
//! Pipes have no size limit and no idle timeout: a quiet command's pipe stays
//! open for as long as its command runs.

use axum::body::Body;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, UPGRADE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum_extra::TypedHeader;
use axum_extra::headers::{Authorization, Range};
use axum_extra::headers::authorization::Bearer;
use demi_backend_database::accounts::TokenHash;
use demi_backend_database::devices::DeviceRecord;
use demi_backend_remote_host::{DeviceSource, PipeRefusal, accept_stream_pipe, offer_response};
use demi_runner_protocol::release::{
    RELEASE_HEADER, RunnerUpdate, TARGET_HEADER, TOKEN_HEADER, UpdateExecutable,
};
use demi_runner_protocol::values::DeviceToken;
use demi_runner_protocol::wire::MAX_MESSAGE_BYTES;
use demi_web_api_protocol::devices::DeviceKind;
use futures_util::StreamExt as _;
use tracing::Instrument as _;

use super::AppState;
use super::error::ApiError;
use super::install::{current_runner_release, range_start};
use super::listener::Peer;
use super::runner_socket::accept;

/// `WS /api/runner`: the socket carries MessagePack frames of at most the
/// wire's limit; a larger one closes it. A backend with runner releases
/// opens it only for a runner of its current release, and answers any other
/// with 409 and the release to update to (`crates/runner/src/update.rs`).
pub(super) async fn socket(
    State(state): State<AppState>,
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
        .max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| accept(state.services, state.shards, socket).instrument(span)))
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

/// The paired device whose token a runner's request carries in
/// `Demi-Runner-Token`: none for a request without one, from a runner of a
/// release before 0.1.14, or with one no paired device holds.
pub(super) async fn paired_device(state: &AppState, headers: &HeaderMap) -> Option<DeviceRecord> {
    let token = headers
        .get(TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| DeviceToken::try_from(value.to_owned()).ok())?;
    match state
        .services
        .control
        .device_by_token(TokenHash::of(token.expose()))
        .await
    {
        Ok(Some(device)) if device.kind == DeviceKind::User => Some(device),
        Ok(_) => None,
        Err(error) => {
            // What the device's runner asked for goes ahead; its device shows
            // nothing of it.
            tracing::warn!(
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
        .shards
        .of(&device.user)
        .call(move |shard, _| async move { shard.runner_updating(&id) })
        .await;
    // A closing backend shows nothing more.
    let _ = shown;
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
        Err(failure) => (StatusCode::CONFLICT, failure.reason().to_owned()).into_response(),
    }
}

/// `GET /api/pipes/:id`: the pipe's bytes for its sink, or, as a WebSocket
/// upgrade, a stream pipe's bytes from its source. A sink's answer goes out
/// at once, its body in the pipe's frames (`crates/runner-protocol/src/wire/pipe_frames.rs`),
/// which say how the pipe ended.
pub(super) async fn get(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<Peer>,
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
    let owner = device.user.clone();
    let offered = state
        .shards
        .of(&owner)
        .call_while_closing(move |shard, _| async move {
            shard.pipes().offers().bytes(&id, device.id.as_str())
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

/// `WS /api/pipes/:id`: a stream pipe's source (`crates/runner-protocol/src/wire/pipe_frames.rs`), relayed into the pipe as `accept_stream_pipe` says. A
/// refusal is an HTTP answer before the upgrade.
async fn stream_source(
    state: &AppState,
    id: String,
    device: DeviceRecord,
    upgrade: WebSocketUpgrade,
) -> Response {
    match claim_source(state, id, device).await {
        Ok(source) => accept_stream_pipe(upgrade, source),
        Err(refusal) => *refusal,
    }
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
