//! A program's process, `demi-core`: it runs the cores a program opens, and
//! serves the program the core's routes as HTTP/2 over the stream the program
//! gives it, its standard input and output.
//!
//! ```text
//! program --- HTTP/2 ---> PUT /plugins {plugins}    the program's plugins
//!                         POST /cores {dir}         opens a core, 201 {id}
//!                         DELETE /cores/{id}        closes it
//!                         /cores/{id}/api/...       the core's routes, as a
//!                                                   page has them under /api
//!                         /hooks/{id}               a schedule's URL
//! ```
//!
//! The process is an isolated instance with one set of services for its
//! cores, as the server has for its users'. They start as the first core
//! opens, with the plugins the program declared until then, as the server's
//! registry is fixed at its start. The program is the caller of each core's
//! routes, and its starter: the core's notifications go on the starter lane
//! the program opens on its page connection to the core.
//!
//! The program's connection is the process's life: as it ends, every core
//! closes, as the server's stop closes its users' cores.

mod cores;
mod hosts;
mod packages;
mod plugins;
mod program;
mod stream;

use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use axum::extract::{OriginalUri, Path, Request, State};
use axum::http::{StatusCode, Uri};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, connect, delete, get, post, put};
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
use demi_plugin_host::Registry;
use demi_plugin_interface::{Manifest, PluginFactory};
use demi_provider_claude_code::releases::DEFAULT_RELEASES_URL;
use demi_provider_common::media_catalog;
use demi_provider_common::models_dev::ModelsDevClient;
use demi_shared_types::{Clock, SystemClock};
use demi_web_api_protocol::connection::CONNECT_ROUTE;
use demi_web_api_protocol::cores::{OpenCore, OpenedCore};
use demi_web_api_protocol::error::ErrorCode;
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
use self::plugins::ProgramPlugin;
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
    cores: Arc<OpenCores>,
    services: Arc<ProcessServices>,
}

/// The process's services, which start as the program's first core opens.
struct ProcessServices {
    public_url: Option<Url>,
    release: Arc<ReleaseFiles>,
    closing: CancellationToken,
    /// The plugins the program declared; the services start with them.
    declared: tokio::sync::Mutex<Vec<Manifest>>,
    started: OnceLock<Started>,
    /// The cores' threads, until the process ends.
    pool: Mutex<Option<ShardPool>>,
}

/// The started services, as the core's routes reach them.
struct Started {
    api: Api,
    /// The core's routes, for the program's calls of a serving core.
    core_routes: Router,
}

/// `PUT /plugins`: the program's plugins, which its cores' agents use
/// besides the release's.
#[derive(Debug, serde::Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeclaredPlugins {
    #[garde(skip)]
    plugins: Vec<Manifest>,
}

/// Runs the process over `io`, the program's connection, until it ends,
/// then closes every core. `public_url` is where the program's server
/// forwards the process's routes; without one, the cores offer neither a
/// plugin's OAuth nor a schedule's URL. `release` holds the roots of the
/// release whose command packages the cores' runners run; without a root,
/// they run none.
pub async fn run<IO>(io: IO, public_url: Option<Url>, release: Vec<PathBuf>) -> Result<(), ProcessError>
where
    IO: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let release = Arc::new(ReleaseFiles::read(release).await?);
    let cores = Arc::new(OpenCores::new(public_url.clone()));
    let services = Arc::new(ProcessServices {
        public_url,
        release,
        closing: CancellationToken::new(),
        declared: tokio::sync::Mutex::default(),
        started: OnceLock::new(),
        pool: Mutex::default(),
    });
    let app = routes(ProcessState {
        cores: cores.clone(),
        services: services.clone(),
    });
    tracing::info!(event = "process.serving", "the process serves its program");
    let served = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
        .enable_connect_protocol()
        .serve_connection(TokioIo::new(io), TowerToHyperService::new(app))
        .await;
    if let Err(error) = served {
        tracing::warn!(event = "process.connection_failed", %error, "the program's connection failed");
    }
    services.closing.cancel();
    // The started services are no route's to start any more: the lock is
    // the start's.
    let _declared = services.declared.lock().await;
    if let Some(started) = services.started.get() {
        cores.close_all(&started.api.services, &started.api.shards).await;
        let pool = services.pool.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(pool) = pool {
            pool.close().await;
        }
        started.api.services.close_providers().await;
    }
    tracing::info!(event = "process.ended", "the process ended with its program's connection");
    Ok(())
}

impl ProcessServices {
    /// The started services, which the first call starts with the
    /// program's plugins.
    async fn start(&self, cores: &Arc<OpenCores>) -> Result<&Started, ProcessError> {
        if let Some(started) = self.started.get() {
            return Ok(started);
        }
        let declared = self.declared.lock().await;
        if let Some(started) = self.started.get() {
            return Ok(started);
        }
        let (started, pool) = self.start_with(cores, &declared).await?;
        *self.pool.lock().unwrap_or_else(PoisonError::into_inner) = Some(pool);
        cores.fix_plugins(declared.iter().map(|manifest| manifest.id.clone()).collect());
        Ok(self.started.get_or_init(|| started))
    }

    /// The process's plugins: the release's, then the program's.
    fn plugins(&self, cores: &Arc<OpenCores>, declared: &[Manifest]) -> Vec<Box<dyn PluginFactory>> {
        let mut plugins = builtins::plugins();
        plugins.extend(declared.iter().map(|manifest| {
            Box::new(ProgramPlugin::new(manifest.clone(), cores.clone())) as Box<dyn PluginFactory>
        }));
        plugins
    }

    async fn start_with(
        &self,
        cores: &Arc<OpenCores>,
        declared: &[Manifest],
    ) -> Result<(Started, ShardPool), ProcessError> {
        let public = PublicUrl::default();
        if let Some(url) = &self.public_url {
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
            plugins: self.plugins(cores, declared),
            packages: Arc::new(ServedPackages(self.release.clone())),
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
            services,
            shards: pool.shards(),
            html: Arc::default(),
            pages: PageTuning::default(),
            schedule_owners: cores.clone(),
        };
        let core_routes = core_routes(api.clone(), self.release.clone(), self.closing.clone());
        tracing::info!(event = "process.started", plugins = declared.len(), "the process's services started");
        Ok((Started { api, core_routes }, pool))
    }
}

/// The core's routes, as the program calls them of a serving core.
fn core_routes(api: Api, release: Arc<ReleaseFiles>, closing: CancellationToken) -> Router {
    crate::routes()
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
        .with_state(api)
        .layer(Extension(release))
        // Each stream is its request's connection.
        .layer(middleware::from_fn(stream::stream_peer))
}

/// The process's routes.
fn routes(state: ProcessState) -> Router {
    Router::new()
        .route("/plugins", put(declare))
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

/// `PUT /plugins`: the program's plugins, 204, until the first core opens;
/// then 409, as the services started with the plugins declared before.
async fn declare(
    State(state): State<ProcessState>,
    JsonBody(declared): JsonBody<DeclaredPlugins>,
) -> Result<StatusCode, ApiError> {
    let services = &state.services;
    let mut held = services.declared.lock().await;
    if services.started.get().is_some() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::PluginsFixed,
            "The process's plugins are fixed: a core has opened",
        ));
    }
    // The registry the services would start with, so that a manifest that
    // breaks a rule is refused here rather than by the first open.
    let packages = ServedPackages(services.release.clone());
    Registry::new(services.plugins(&state.cores, &declared.plugins), |operation| {
        demi_core_hosts::packages::Packages::serves(&packages, &operation.package, &[operation.operation.as_str()])
    })
    .map_err(|error| ApiError::invalid_body(error.to_string()))?;
    *held = declared.plugins;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /cores`: opens the core in `dir`, 201 with its id. The first open
/// starts the process's services.
async fn open(
    State(state): State<ProcessState>,
    JsonBody(open): JsonBody<OpenCore>,
) -> Result<(StatusCode, Json<OpenedCore>), ApiError> {
    let started = state.services.start(&state.cores).await.map_err(|error| {
        tracing::error!(event = "process.start_failed", %error, "the process's services did not start");
        ApiError::internal(&error)
    })?;
    let id = state
        .cores
        .open(&PathBuf::from(open.dir), &started.api.services, &started.api.shards)
        .await?;
    Ok((StatusCode::CREATED, Json(OpenedCore { id })))
}

/// `DELETE /cores/{id}`: closes the core, 204.
async fn close(State(state): State<ProcessState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let id = UserId::try_from(id).map_err(|_| not_open())?;
    let started = state.services.started.get().ok_or_else(not_open)?;
    state.cores.close(&id, &started.api.services, &started.api.shards).await?;
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
    // A serving core opened after the start.
    let started = state.services.started.get().ok_or_else(not_open)?;
    let path = match request.uri().query() {
        Some(query) => format!("/{place}?{query}"),
        None => format!("/{place}"),
    };
    *request.uri_mut() = path.parse::<Uri>().map_err(|_| not_open())?;
    let caller = ProgramCaller::new(id, core, state.cores.clone());
    request.extensions_mut().insert(CallerOf(Arc::new(caller)));
    // The router answers every request.
    Ok(started
        .core_routes
        .clone()
        .oneshot(request)
        .await
        .unwrap_or_else(|never| match never {}))
}

/// `/hooks/{id}`: a call of a schedule's URL, which reaches the serving core
/// that holds the schedule; before the start, none does.
async fn hook(State(state): State<ProcessState>, id: Path<String>, request: Request) -> Result<Response, ApiError> {
    let Some(started) = state.services.started.get() else {
        return Err(crate::schedules::unauthorized());
    };
    crate::schedules::hook(State(started.api.clone()), id, request).await
}

pub(super) async fn no_route(request: Request) -> Response {
    // The core's routes see their own path; the program called the whole.
    let path = match request.extensions().get::<OriginalUri>() {
        Some(OriginalUri(uri)) => uri.path().to_owned(),
        None => request.uri().path().to_owned(),
    };
    ApiError::no_route(request.method(), &path).into_response()
}
