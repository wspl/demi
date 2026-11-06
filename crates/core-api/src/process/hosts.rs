//! A program's Hosts: it makes one, then carries its runner's bytes to the
//! process over a channel of the program's own, such as `docker exec` of a
//! `demi-runner run --stdio`.
//!
//! ```text
//! runner's stdio <-- bytes --> program <-- CONNECT .../devices/{id}/runner ->
//!                                          :protocol h2c, HTTP/2 inside:
//!                                          the runner's socket and pipes
//! ```
//!
//! The channel names the device, so the runner presents no token; the
//! runner's routes are the ones the server serves a runner whose token names
//! its device.

use std::sync::Arc;

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::{Extension, Json, Router};
use demi_core_hosts::devices::RunnerSocket;
use demi_core_hosts::packages::ARTIFACT_ROUTE;
use demi_runner_protocol::wire::{LINK_FRAME_BYTES, Watched, WriteProgress};
use demi_web_api_protocol::devices::{DeviceAnswer, DeviceKind};
use hyper::ext::Protocol;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::service::TowerToHyperService;
use tracing::Instrument as _;

use super::packages::ReleaseFiles;
use super::stream::stream_peer;
use crate::api::Api;
use crate::caller::Principal;
use crate::error::ApiError;
use crate::runners::{RunnerDevice, RunnerOf};

/// `POST /devices`: makes a Host, 201 with it.
pub(crate) async fn create(
    State(state): State<Api>,
    Principal(user): Principal,
) -> Result<(StatusCode, Json<DeviceAnswer>), ApiError> {
    let device = state
        .shards
        .of(&user)
        .call(|core, _| async move { core.create_host().await })
        .await??;
    Ok((StatusCode::CREATED, Json(DeviceAnswer { device })))
}

/// `CONNECT /devices/{id}/runner`, with `:protocol` `h2c`: the Host's
/// channel, on which the process serves its runner's routes once it
/// answers 200.
pub(crate) async fn channel(
    State(state): State<Api>,
    Principal(owner): Principal,
    Path(device): Path<String>,
    Extension(release): Extension<Arc<ReleaseFiles>>,
    mut request: Request,
) -> Result<Response, ApiError> {
    let protocol = request.extensions().get::<Protocol>().map(Protocol::as_str);
    if protocol != Some(CHANNEL_PROTOCOL) {
        return Ok(super::no_route(request).await);
    }
    let record = crate::devices::owned_device(&state, &owner, &device, Some(DeviceKind::User)).await?;
    let upgrade = hyper::upgrade::on(&mut request);
    let device = RunnerDevice { owner, id: record.id };
    let span = tracing::info_span!("runner_channel", host_id = %device.id);
    tokio::spawn(
        async move {
            let io = match upgrade.await {
                Ok(io) => io,
                Err(error) => {
                    tracing::info!(event = "runner_channel.not_opened", %error, "a runner's channel did not open");
                    return;
                }
            };
            tracing::info!(event = "runner_channel.opened", "a runner's channel opened");
            // The channel is the runner's connection, as its TCP connection
            // is the server's: its link counts it lost once its writes stall.
            let (io, writes) = Watched::new(TokioIo::new(io));
            let routes = runner_routes(state, device, release, ChannelWrites(writes));
            let served = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                .enable_connect_protocol()
                .serve_connection(TokioIo::new(io), TowerToHyperService::new(routes))
                .await;
            match served {
                Ok(()) => tracing::info!(event = "runner_channel.closed", "a runner's channel closed"),
                Err(error) => tracing::info!(event = "runner_channel.closed", %error, "a runner's channel broke"),
            }
        }
        .instrument(span),
    );
    Ok(StatusCode::OK.into_response())
}

/// The channel's `:protocol`: HTTP/2 without TLS (RFC 9113 § 3.1), which
/// the channel carries.
const CHANNEL_PROTOCOL: &str = "h2c";

/// The writes of a Host's channel.
#[derive(Clone)]
struct ChannelWrites(Arc<WriteProgress>);

/// The routes a Host's channel serves its runner, as the device `device`.
fn runner_routes(api: Api, device: RunnerDevice, release: Arc<ReleaseFiles>, writes: ChannelWrites) -> Router {
    crate::runners::routes()
        .route("/api/runner", any(socket))
        .route(&format!("{ARTIFACT_ROUTE}/{{sha256}}"), get(super::packages::artifact))
        .fallback(super::no_route)
        .with_state(api)
        .layer(Extension(RunnerOf(device)))
        .layer(Extension(release))
        .layer(Extension(writes))
        .layer(middleware::from_fn(stream_peer))
        .layer(middleware::from_fn(crate::request_id::assign))
}

/// `WS /api/runner`: the runner's link, which its core takes once its hello
/// came.
async fn socket(
    State(state): State<Api>,
    RunnerOf(device): RunnerOf,
    Extension(ChannelWrites(writes)): Extension<ChannelWrites>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let deadline = state.services.runners.hello_deadline;
    let shards = state.shards.clone();
    let span = tracing::Span::current();
    upgrade
        .max_message_size(LINK_FRAME_BYTES)
        .max_frame_size(LINK_FRAME_BYTES)
        .on_upgrade(move |socket| {
            let socket = RunnerSocket::new(socket, writes);
            crate::runner_socket::accept_device(deadline, shards, device.owner, device.id, socket).instrument(span)
        })
}
