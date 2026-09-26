//! The HTTP edge (`backend.md` § Runtime model): the listener and router, the
//! session gate, request extractors and body limits, error codes and the
//! browser build. Handlers are thin: they parse, authenticate, and call a
//! shared service or a shard.

mod accounts;
mod assets;
mod attachments;
mod auth;
mod blobs;
mod browser;
mod body;
mod cloud;
mod content;
mod conversations;
mod cookies;
mod devices;
mod error;
mod expose;
mod exposes;
mod files;
mod gate;
mod hosts;
mod install;
mod listener;
mod models;
mod panel;
mod provider_cli;
mod providers;
mod query;
mod runners;
mod settings;
mod sidebar;
mod state;
mod streams;
mod transfer;
mod usage;
mod users;
mod workspaces;

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, DefaultBodyLimit, FromRef, OriginalUri, Request, State};
use axum::http::Method;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use hyper::body::Incoming;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt as _;
use tower_http::trace::TraceLayer;

use self::error::ApiError;
pub(crate) use self::install::Site;
use self::listener::{EdgeListener, Peer};
use crate::backend::Services;
use crate::shard::Shards;

/// The listener and the server over it.
pub(crate) struct Edge {
    local_addr: SocketAddr,
    /// Cancelled when shutdown starts: the listener closes, and requests on
    /// open connections answer 503.
    closing: CancellationToken,
    /// Cancelled to close every connection still open.
    connections: CancellationToken,
    /// Cancelled to end the server once its connections are gone.
    stop: CancellationToken,
    serving: JoinHandle<()>,
}

impl Edge {
    pub(crate) async fn start(
        address: SocketAddr,
        state: AppState,
        web_directory: Option<PathBuf>,
    ) -> io::Result<Self> {
        let tcp = TcpListener::bind(address).await?;
        let local_addr = tcp.local_addr()?;
        // Before the first request: a Cloud's boot names this URL.
        state.services.cloud.listening(state.site.public_url.as_ref(), local_addr);
        let closing = CancellationToken::new();
        let connections = CancellationToken::new();
        let stop = CancellationToken::new();
        let listener = EdgeListener::new(tcp, closing.clone(), connections.clone());
        let app = router(state.clone(), closing.clone(), web_directory);
        let serving = tokio::spawn(listener::serve(listener, stop.clone(), move |peer, request| {
            respond(app.clone(), state.clone(), peer, request)
        }));
        Ok(Self {
            local_addr,
            closing,
            connections,
            stop,
            serving,
        })
    }

    pub(crate) fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Closes the listener. Open connections go on serving, but a new
    /// request on one answers 503 `backend_closing`.
    pub(crate) fn stop_accepting(&self) {
        self.closing.cancel();
    }

    /// Closes the connections still open, such as a download's, and waits
    /// for the server to end.
    pub(crate) async fn close(self) -> io::Result<()> {
        self.closing.cancel();
        self.connections.cancel();
        self.stop.cancel();
        self.serving.await.map_err(io::Error::other)
    }
}

/// Answers one request: an expose hostname's with the public relay, before
/// any route sees it, and every other with the routes.
async fn respond(app: Router, state: AppState, peer: Peer, mut request: Request<Incoming>) -> Response {
    if let Some(label) = expose::expose_label(&state, &request) {
        return expose::relay(state, peer, label, request).await;
    }
    request.extensions_mut().insert(ConnectInfo(peer));
    match app.oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    }
}

/// What the routes reach: the services every request may use, the users'
/// shards, and how the backend is reached from outside.
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) services: Arc<Services>,
    pub(crate) shards: Shards,
    pub(crate) site: Arc<Site>,
}

impl FromRef<AppState> for Arc<Services> {
    fn from_ref(state: &AppState) -> Self {
        state.services.clone()
    }
}

impl FromRef<AppState> for Shards {
    fn from_ref(state: &AppState) -> Self {
        state.shards.clone()
    }
}

/// The routes. Every `/api` path, unknown paths included, passes the session
/// gate first, except those outside `session_api`: the public entrances,
/// setup and login, and the routes that authenticate with device
/// credentials instead. The pipes stay outside the 503 of a closing backend,
/// whose shutdown needs them, and outside the body limit, since their bodies
/// have none.
fn router(state: AppState, closing: CancellationToken, web_directory: Option<PathBuf>) -> Router {
    let entrances = Router::new()
        .route("/install.sh", get(install::shell))
        .route("/install.ps1", get(install::powershell))
        .route("/runner-artifacts/{release}/{target}/{file}", get(install::artifact))
        .route("/api/setup", get(auth::setup_status).post(auth::setup))
        .route("/api/auth/login", post(auth::login))
        .route("/api/runner", get(runners::socket))
        .method_not_allowed_fallback(no_route);
    let pipes = Router::new()
        .route("/api/pipes/{id}", put(runners::put).get(runners::get))
        .method_not_allowed_fallback(no_route);
    let session_api = Router::new()
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me).patch(auth::set_nickname))
        .route("/auth/password", put(auth::change_password))
        .route("/auth/email", post(auth::start_email_change))
        .route("/auth/email/confirm", post(auth::confirm_email_change))
        .route("/state", get(state::state))
        .route("/settings", get(settings::settings))
        .route(
            "/settings/preferences",
            get(settings::preferences).patch(settings::patch_preferences),
        )
        .route(
            "/attachments",
            post(attachments::upload).layer(DefaultBodyLimit::max(attachments::BODY_LIMIT)),
        )
        .route("/blobs/{sha256}", get(blobs::blob))
        .route("/models", get(models::models))
        .route("/providers", get(providers::list).post(providers::create))
        .route("/providers/catalog", get(providers::catalog))
        .route("/providers/setup-token", post(accounts::import_setup_token))
        .route("/providers/subscription-login", post(accounts::start_login))
        .route(
            "/providers/subscription-login/{id}",
            get(accounts::login_state).delete(accounts::cancel_login),
        )
        .route("/providers/{id}", patch(providers::update).delete(providers::delete))
        .route("/providers/{id}/status", get(providers::status))
        .route("/providers/{id}/quota", post(providers::quota))
        .route("/providers/{id}/test", post(providers::test))
        .route("/providers/{id}/cli", get(provider_cli::read))
        .route("/providers/{id}/cli/install", post(provider_cli::install))
        .route("/providers/{id}/accounts", get(accounts::list).post(accounts::add_token))
        .route("/providers/{id}/accounts/active", put(accounts::activate))
        .route("/providers/{id}/accounts/login", post(accounts::login_into))
        .route("/providers/{id}/accounts/{credential}", delete(accounts::remove))
        .route("/users", get(users::list).post(users::create))
        .route("/users/{id}", patch(users::reset_password))
        .route("/usage", get(usage::totals))
        .route("/usage/instance", get(usage::instance))
        .route("/conversations", get(conversations::list).post(conversations::create))
        .route("/conversations/batch", post(conversations::batch))
        .route("/conversations/{id}", patch(conversations::patch))
        .route("/conversations/{id}/fork", post(conversations::fork))
        .route("/conversations/{id}/title", post(conversations::title))
        .route("/conversations/{id}/transcript", get(conversations::transcript))
        .route("/conversations/{id}/read", post(conversations::read))
        .route("/conversations/{id}/stream", get(conversations::stream))
        .route("/conversations/{id}/streams/{name}", get(streams::open))
        .route("/conversations/{id}/panel", get(panel::read).put(panel::save))
        .route("/conversations/{id}/browser/tabs", get(browser::list).post(browser::open))
        .route("/conversations/{id}/browser/tabs/{tab}", delete(browser::close))
        .route("/conversations/{id}/browser/tabs/{tab}/navigate", post(browser::navigate))
        .route("/conversations/{id}/browser/tabs/{tab}/history", post(browser::history))
        .route("/sidebar/reorder", post(sidebar::reorder))
        .route("/workspaces", get(workspaces::list).post(workspaces::create))
        .route("/workspaces/{id}", patch(workspaces::rename).delete(workspaces::delete))
        .route("/cloud", get(cloud::status))
        .route("/cloud/reset", post(cloud::reset))
        .route("/exposes", get(exposes::list).post(exposes::create))
        .route("/exposes/{id}", delete(exposes::remove))
        .route("/exposes/{id}/renew", post(exposes::renew))
        .route("/devices", get(devices::list))
        .route("/devices/claim", post(devices::claim))
        .route("/devices/{id}", delete(devices::revoke))
        .route("/devices/{id}/fs", get(devices::browse).post(devices::make_directory))
        .route("/devices/{id}/log", get(devices::log))
        .route(
            "/conversations/{id}/fs",
            get(files::list).post(files::make_directory).delete(files::remove),
        )
        .route("/conversations/{id}/fs/file", get(files::text))
        .route("/conversations/{id}/fs/raw", get(files::raw).put(files::upload))
        .route("/conversations/{id}/hosts", get(hosts::list).post(hosts::attach))
        .route(
            "/conversations/{id}/hosts/{device}",
            patch(hosts::rename).delete(hosts::detach),
        )
        .route(
            "/conversations/{id}/hosts/{device}/fs",
            get(files::list_on_host).post(files::make_directory_on_host),
        )
        .route("/conversations/{id}/changes", get(files::changes))
        .route("/conversations/{id}/changes/file", get(files::changed_file))
        .route("/conversations/{id}/changes/raw", get(files::committed))
        .route("/conversations/{id}/commands/{command}/changes/file", get(files::retained_edit))
        .fallback(no_route)
        .method_not_allowed_fallback(no_route)
        // After the fallbacks, so the gate covers them too.
        .layer(middleware::from_fn_with_state(state.services.clone(), gate::session));
    let app = Router::new().merge(entrances).nest("/api", session_api);
    let app = match web_directory {
        Some(directory) => assets::serve(app, directory),
        None => app.fallback(no_route),
    };
    app.layer(DefaultBodyLimit::max(body::JSON_BODY_LIMIT))
        .layer(middleware::from_fn_with_state(closing, refuse_while_closing))
        .merge(pipes)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// The JSON 404 of a method and path no route answers.
async fn no_route(method: Method, OriginalUri(uri): OriginalUri) -> ApiError {
    ApiError::no_route(&method, uri.path())
}

/// Answers 503 `backend_closing` once shutdown starts. Shutdown itself needs
/// the runners' pipes, so the routes that carry them stay outside this
/// layer.
async fn refuse_while_closing(State(closing): State<CancellationToken>, request: Request, next: Next) -> Response {
    if closing.is_cancelled() {
        return ApiError::backend_closing().into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::StatusCode;
    use demi_web_api::error::ErrorCode;

    use super::*;
    use crate::shard::{ShardPlacement, ShardPool};

    #[tokio::test]
    async fn a_request_after_shutdown_started_answers_backend_closing() {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let pool = ShardPool::start(ShardPlacement::Threads(std::num::NonZeroUsize::MIN), services.clone())
            .await
            .unwrap();
        let state = AppState {
            services,
            shards: pool.shards(),
            site: Arc::default(),
        };
        let closing = CancellationToken::new();
        let app = router(state, closing.clone(), None);
        let request = || Request::get("/api/setup").body(Body::empty()).unwrap();
        assert_eq!(app.clone().oneshot(request()).await.unwrap().status(), StatusCode::OK);

        closing.cancel();
        let refused = app.oneshot(request()).await.unwrap();
        assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = axum::body::to_bytes(refused.into_body(), usize::MAX).await.unwrap();
        let error: demi_web_api::error::ErrorBody = serde_json::from_slice(&body).unwrap();
        assert_eq!(error.code, ErrorCode::BackendClosing);
        pool.close().await;
    }
}
