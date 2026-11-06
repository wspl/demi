//! A program's process, `demi-core`: it runs the cores a program opens, and
//! serves the program the core's routes as HTTP/2 over the stream the program
//! gives it, its standard input and output.
//!
//! ```text
//! program --- HTTP/2 ---> POST /cores {dir}         opens a core, 201 {id}
//!                         DELETE /cores/{id}        closes it
//!                         /cores/{id}/api/...       the core's routes, as a
//!                                                   page has them under /api
//!                         /hooks/{id}               a schedule's URL
//! ```
//!
//! The process is an isolated instance with one set of services for its
//! cores, as the server has for its users'. The program is the caller of
//! each core's routes, and its starter: the core's notifications go on the
//! starter lane the program opens on its page connection to the core.
//!
//! The program's connection is the process's life: as it ends, every core
//! closes, as the server's stop closes its users' cores.

mod cores;
mod hosts;
mod packages;
mod program;
mod stream;

use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{OriginalUri, Path, Request, State};
use axum::http::{StatusCode, Uri};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, connect, delete, get, post};
use axum::{Extension, Json, Router};
use demi_core::builtins::{self, RelayFamily};
use demi_core::conversation::claude_cli::MachineSlot;
use demi_core::pool::{ShardPlacement, ShardPool};
use demi_core::services::{CoreServices, CoreServicesError, CoreSettings};
use demi_core::tuning::{ConversationTuning, LifecycleTuning, MediaTuning, RunnerTuning};
use demi_core_blobs::blobs::BlobStores;
use demi_core_hosts::packages::ReleaseError;
use demi_core_hosts::public_url::PublicUrl;
use demi_core_providers::vault::logins::LoginTiming;
use demi_provider_claude_code::releases::DEFAULT_RELEASES_URL;
use demi_provider_common::media_catalog;
use demi_provider_common::models_dev::ModelsDevClient;
use demi_shared_types::{Clock, SystemClock};
use demi_web_api_protocol::connection::CONNECT_ROUTE;
use demi_web_api_protocol::cores::{OpenCore, OpenedCore};
use demi_web_api_protocol::ids::UserId;
use demi_web_api_protocol::settings::InstanceMode;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::service::TowerToHyperService;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt as _;
use tower_http::trace::TraceLayer;
use url::Url;

use self::cores::{OpenCores, ProcessOpener, not_open};
use self::packages::{ReleaseFiles, ServedPackages};
use self::program::ProgramCaller;
use crate::api::Api;
use crate::body::JsonBody;
use crate::caller::CallerOf;
use crate::connection::{Closing, PageTuning};
use crate::error::ApiError;

/// Why the process did not run.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error(transparent)]
    Services(#[from] CoreServicesError),
    #[error("the cores' threads did not start: {0}")]
    Threads(std::io::Error),
    #[error(transparent)]
    Release(#[from] ReleaseError),
}

/// What the routes of the process reach.
#[derive(Clone)]
struct ProcessState {
    api: Api,
    cores: Arc<OpenCores>,
    /// The core's routes, for the program's calls of a serving core.
    core_routes: Router,
}

/// Runs the process over `io`, the program's connection, until it ends,
/// then closes every core. `public_url` is where the program's server
/// forwards the process's routes; without one, the cores offer neither a
/// plugin's OAuth nor a schedule's URL. `release` is the release whose
/// command packages the cores' runners run; without one, they run none.
pub async fn run<IO>(io: IO, public_url: Option<Url>, release: Option<PathBuf>) -> Result<(), ProcessError>
where
    IO: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let release = Arc::new(ReleaseFiles::read(release).await?);
    let cores = Arc::new(OpenCores::new(public_url.clone()));
    let public = PublicUrl::default();
    if let Some(url) = &public_url {
        // A URL given is the one taken; the address goes unused.
        public.listening(Some(url), SocketAddr::from(([127, 0, 0, 1], 0)));
    }
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let claude_machine = MachineSlot::default();
    let settings = CoreSettings {
        families: builtins::families(claude_machine.clone())
            .with(demi_provider_relay::borrow::FAMILY, RelayFamily(public.clone())),
        public_url: public,
        clock: clock.clone(),
        plugins: builtins::plugins(),
        packages: Arc::new(ServedPackages(release.clone())),
        models_dev_url: ModelsDevClient::DEFAULT_URL
            .parse()
            .expect("the models.dev address parses"),
        model_shutdowns_url: demi_provider_registry::shutdowns::DEFAULT_URL
            .parse()
            .expect("LiteLLM's address parses"),
        media_catalog_url: media_catalog::branch_url(option_env!("DEMI_BUILD_BRANCH")),
        media_catalog: media_catalog::Catalog::carried(),
        claude_releases: DEFAULT_RELEASES_URL
            .parse()
            .expect("the Claude Code distribution's address parses"),
        claude_machine,
        logins: LoginTiming::default(),
        runners: RunnerTuning::default(),
        conversations: ConversationTuning::default(),
        media: MediaTuning::default(),
        lifecycle: LifecycleTuning::default(),
        stun: Vec::new(),
    };
    let services = CoreServices::start(
        settings,
        cores.clone(),
        InstanceMode::Isolated,
        false,
        BlobStores::own(clock),
    )
    .await?;
    let services = Arc::new(services);
    let pool = match ShardPool::start(
        ShardPlacement::Threads(NonZeroUsize::MIN),
        services.clone(),
        Arc::new(ProcessOpener(cores.clone())),
    )
    .await
    {
        Ok(pool) => pool,
        Err(error) => {
            services.close_providers().await;
            return Err(ProcessError::Threads(error));
        }
    };
    let api = Api {
        services: services.clone(),
        shards: pool.shards(),
        html: Arc::default(),
        pages: PageTuning::default(),
        schedule_owners: cores.clone(),
    };
    let closing = CancellationToken::new();
    let app = routes(api.clone(), cores.clone(), release, closing.clone());
    tracing::info!(event = "process.serving", "the process serves its program");
    let served = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
        .enable_connect_protocol()
        .serve_connection(TokioIo::new(io), TowerToHyperService::new(app))
        .await;
    if let Err(error) = served {
        tracing::warn!(event = "process.connection_failed", %error, "the program's connection failed");
    }
    closing.cancel();
    cores.close_all(&services, &api.shards).await;
    pool.close().await;
    services.close_providers().await;
    tracing::info!(event = "process.ended", "the process ended with its program's connection");
    Ok(())
}

/// The process's routes.
fn routes(api: Api, cores: Arc<OpenCores>, release: Arc<ReleaseFiles>, closing: CancellationToken) -> Router {
    let core_routes = crate::routes()
        .route("/state", get(crate::state::read))
        // The reads' caching (`revalidate.rs`): the routes' answers only.
        .layer(middleware::from_fn(crate::revalidate::reads))
        .route(
            CONNECT_ROUTE,
            any(crate::connection::connect).layer(Extension(Closing(closing))),
        )
        .merge(crate::public_routes())
        .route("/devices", post(hosts::create))
        .route("/devices/{id}/runner", connect(hosts::channel))
        .fallback(no_route)
        .with_state(api.clone())
        .layer(Extension(release))
        // Each stream is its request's connection.
        .layer(middleware::from_fn(stream::stream_peer));
    let state = ProcessState {
        api,
        cores,
        core_routes,
    };
    Router::new()
        .route("/cores", post(open))
        .route("/cores/{id}", delete(close))
        .route("/hooks/{id}", any(hook))
        // A route with parameters would give them to the core's routes too.
        .fallback(core_call)
        .with_state(state)
        .layer(axum::extract::DefaultBodyLimit::max(crate::body::JSON_BODY_LIMIT))
        .layer(middleware::from_fn(crate::request_id::watch_abandon))
        // The request's line is `request_id::answered`'s alone.
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request| crate::request_id::span(request))
                .on_response(crate::request_id::answered)
                .on_failure(()),
        )
        // Outermost, so every answer carries the request's ID.
        .layer(middleware::from_fn(crate::request_id::assign))
}

/// `POST /cores`: opens the core in `dir`, 201 with its id.
async fn open(
    State(state): State<ProcessState>,
    JsonBody(open): JsonBody<OpenCore>,
) -> Result<(StatusCode, Json<OpenedCore>), ApiError> {
    let id = state
        .cores
        .open(&PathBuf::from(open.dir), &state.api.services, &state.api.shards)
        .await?;
    Ok((StatusCode::CREATED, Json(OpenedCore { id })))
}

/// `DELETE /cores/{id}`: closes the core, 204.
async fn close(State(state): State<ProcessState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let id = UserId::try_from(id).map_err(|_| not_open())?;
    state.cores.close(&id, &state.api.services, &state.api.shards).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `/cores/{id}/api/...`: the program's call of the serving core `id`, which
/// the core's routes answer as the server's answer a page's under `/api`.
async fn core_call(State(state): State<ProcessState>, mut request: Request) -> Result<Response, ApiError> {
    let Some((id, place)) = request
        .uri()
        .path()
        .strip_prefix("/cores/")
        .and_then(|rest| rest.split_once("/api/"))
    else {
        return Ok(no_route(request).await);
    };
    let id = UserId::try_from(id.to_owned()).map_err(|_| not_open())?;
    let core = state.cores.serving(&id).ok_or_else(not_open)?;
    let path = match request.uri().query() {
        Some(query) => format!("/{place}?{query}"),
        None => format!("/{place}"),
    };
    *request.uri_mut() = path.parse::<Uri>().map_err(|_| not_open())?;
    let caller = ProgramCaller::new(id, core, state.cores.clone());
    request.extensions_mut().insert(CallerOf(Arc::new(caller)));
    // The router answers every request.
    Ok(state.core_routes.oneshot(request).await.unwrap_or_else(|never| match never {}))
}

/// `/hooks/{id}`: a call of a schedule's URL, which reaches the serving core
/// that holds the schedule.
async fn hook(State(state): State<ProcessState>, id: Path<String>, request: Request) -> Result<Response, ApiError> {
    crate::schedules::hook(State(state.api), id, request).await
}

pub(super) async fn no_route(request: Request) -> Response {
    // The core's routes see their own path; the program called the whole.
    let path = match request.extensions().get::<OriginalUri>() {
        Some(OriginalUri(uri)) => uri.path().to_owned(),
        None => request.uri().path().to_owned(),
    };
    ApiError::no_route(request.method(), &path).into_response()
}
