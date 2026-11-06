//! The server's ways in for its runners: `WS /api/runner`, the link a
//! runner's token or a claimed pairing code gives a core, and the device
//! token that finds the device of every request of a runner's pipes
//! (`crates/core-api/src/runners.rs`).

use axum::extract::{ConnectInfo, Request, State};
use axum::extract::ws::WebSocketUpgrade;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::TypedHeader;
use axum_extra::headers::Authorization;
use axum_extra::headers::authorization::Bearer;
use demi_backend_database::accounts::TokenHash;
use demi_core_api::error::ApiError;
use demi_core_api::peer::Peer;
use demi_core_api::runners::{RunnerDevice, RunnerOf};
use demi_core_hosts::devices::RunnerSocket;
use demi_core_store::StorageError;
use demi_runner_protocol::release::{RELEASE_HEADER, RunnerUpdate, TARGET_HEADER, TOKEN_HEADER, UpdateExecutable};
use demi_runner_protocol::values::DeviceToken;
use demi_runner_protocol::wire::LINK_FRAME_BYTES;
use demi_web_api_protocol::devices::DeviceKind;
use tracing::Instrument as _;

use super::AppState;
use super::install::current_runner_release;
use super::runner_socket::accept;

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

/// The paired device whose token a runner's request carries in
/// `Demi-Runner-Token`: none for a request without one, from a runner of a
/// release before 0.1.14, or with one no paired device holds.
pub(super) async fn paired_device(state: &AppState, headers: &HeaderMap) -> Option<RunnerDevice> {
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
        Ok(Some((owner, device))) if device.kind == DeviceKind::User => Some(RunnerDevice { owner, id: device.id }),
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


/// Finds the device whose token a request of a runner's pipes bears; 401
/// without one the backend issued.
pub(super) async fn token_device(
    State(state): State<AppState>,
    authorization: Option<TypedHeader<Authorization<Bearer>>>,
    mut request: Request,
    next: Next,
) -> Response {
    let unauthorized = || (StatusCode::UNAUTHORIZED, "device token required").into_response();
    let Some(TypedHeader(Authorization(bearer))) = authorization else {
        return unauthorized();
    };
    match state.services.control.device_by_token(TokenHash::of(bearer.token())).await {
        Ok(Some((id, owner))) => {
            request.extensions_mut().insert(RunnerOf(RunnerDevice { owner, id }));
            next.run(request).await
        }
        Ok(None) => unauthorized(),
        Err(error) => {
            tracing::error!(
                event = "runner_device.read_failed",
                route = "pipe",
                error = &error as &dyn std::error::Error,
                "a pipe's device could not be read"
            );
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
