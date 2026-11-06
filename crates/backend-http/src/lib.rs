//! The HTTP edge: the listener and router, the session gate, request
//! extractors and body limits, error codes and the web app build.
//!
//! Handlers are thin: they parse, authenticate, check ownership, and call a
//! shared service or a core. That is why the edge is axum, whose requirement
//! that a handler's future be `Send` therefore costs nothing, and whose
//! extractors, tower middleware and socket-free router tests come with it.
//!
//! # Ownership
//!
//! The edge checks ownership before it hands a request to a core: it resolves
//! the caller from the cookie, loads the conversation, device, project or
//! provider entry the path names, and answers 404 when it belongs to someone
//! else. A request therefore reaches only its owner's core, which checks a
//! conversation's owner again whenever it loads the record.
//!
//! # Runners
//!
//! - A runner authenticates with its device token in the first message of its
//!   socket, or as a bearer token on a pipe request.
//! - An unpaired runner waits here for its user to claim the code it printed.
//! - A Cloud guest always presents its token and never pairs.
//!
//! Web sessions and the `Origin` check are the `gate`'s.

mod assets;
mod auth;
mod cloud;
mod cookies;
mod diagnostics;
mod gate;
mod install;
mod lending;
mod listener;
mod manage;
mod pairing;
mod passkeys;
mod push;
mod runner_socket;
mod runners;
mod server_upgrade;
mod session_caller;
mod sessions;
mod settings;
mod shared_blocks;
mod shares;
mod state;
mod update;
mod usage;
mod users;

pub use manage::manage_socket;
pub use shared_blocks::BlockLinks;

use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, DefaultBodyLimit, FromRef, OriginalUri, Request, State};
use axum::Extension;
use axum::http::Method;
use demi_web_api_protocol::connection::CONNECT_ROUTE;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use demi_backend_runners::native;
use hyper::body::Incoming;
use axum::serve::{Listener as _, ListenerExt as _};
use std::os::unix::fs::PermissionsExt as _;
use tokio::net::{TcpListener, UnixListener};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt as _;
use tower_http::trace::TraceLayer;

use demi_core_api::error::ApiError;
pub use self::assets::{WebBuildError, web_build};
pub use self::install::Site;
use self::listener::EdgeListener;
use demi_core_api::peer::Peer;
use demi_backend_user_shard::services::Services;
use demi_core::pool::Shards;
use demi_core_api::api::Api;
use demi_core_api::connection::{self, Closing};

/// The listener and the server over it.
pub struct Edge {
    local_addr: SocketAddr,
    /// Cancelled when shutdown starts: the listener closes, and requests on
    /// open connections answer 503.
    closing: CancellationToken,
    /// Cancelled to close every connection still open.
    connections: CancellationToken,
    /// Cancelled to end the server once its connections are gone.
    stop: CancellationToken,
    serving: JoinHandle<()>,
    /// The runner socket's server, when the backend has the socket.
    serving_runners: Vec<JoinHandle<()>>,
    /// The management socket's server, when the backend has the socket.
    serving_management: Option<JoinHandle<()>>,
}

/// Opens the backend's runner socket at `path`, replacing the file a previous
/// backend left. None when the socket's directory does not exist, as on a
/// machine that runs no Cloud: no Cloud can then reach this backend.
///
/// Anyone who reaches the file may connect, since a runner proves itself with
/// its device token, so the socket is mode 0666 in a directory of mode 0755.
///
/// A Cloud's runner reaches its backend through this socket and never over the
/// network, which would only add a way to fail: behind a CDN a Cloud's
/// connection left the server for the CDN's edge and came back, and the edge
/// reset it every hour or two, which cut off what the runner was sending.
///
/// The socket serves only what a runner uses: the runner socket, the pipes and
/// the downloads of runners and command artifacts. A browser's routes answer
/// 404 there.
///
/// The machine manager mounts the socket's directory read-only at
/// `/run/demi-backend` at every boot, so the socket a restarted backend makes
/// again reaches the running Clouds (the unit's `RuntimeDirectoryPreserve`
/// keeps the directory). The setting's file name must therefore be
/// `runners.sock`, the name a Cloud's runner looks for, in a directory of its
/// own.
///
/// The boot record's backend URL still names the backend's origin, which
/// requests carry as their host:
///
/// - No TLS crosses the socket.
/// - An artifact URL on that origin goes through it.
/// - Any other URL, such as an S3 store's, goes over the network.
///
/// A Simple Isolation Cloud, which runs on the host's network, reaches the
/// backend on an abstract socket named after the file instead
/// (`demi_runner_protocol::boot::abstract_socket`): a bound file would be
/// one its programs could remove.
pub fn runner_socket(path: &Path) -> io::Result<Option<RunnerSockets>> {
    let directory = path.parent().unwrap_or(Path::new("/"));
    if !directory.is_dir() {
        tracing::warn!(
            event = "runner_socket.missing",
            directory = %directory.display(),
            "Clouds cannot reach this backend: the runner socket's directory does not exist"
        );
        return Ok(None);
    }
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let file = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))?;
    #[cfg(target_os = "linux")]
    let named = Some(UnixListener::bind(demi_runner_protocol::boot::abstract_socket_path(
        &demi_runner_protocol::boot::abstract_socket(path),
    ))?);
    #[cfg(not(target_os = "linux"))]
    let named = None;
    Ok(Some(RunnerSockets { file, named }))
}

/// The backend's runner sockets: its file, and on Linux the abstract socket
/// named after it.
pub struct RunnerSockets {
    file: UnixListener,
    named: Option<UnixListener>,
}

/// The network listener of a backend that does not serve yet: it answers
/// what [`update`] does until [`Listening::open`] gives it every route.
pub struct Listening {
    local_addr: SocketAddr,
    routes: Arc<std::sync::RwLock<Router>>,
    closing: CancellationToken,
    connections: CancellationToken,
    stop: CancellationToken,
    serving: JoinHandle<()>,
}

impl Edge {
    /// Listens at `address` before the backend opens its data, answering the
    /// progress of `update`, the web app's files and 503 to the rest. A
    /// share's page names the backend at `public_url`.
    pub async fn listen(
        address: SocketAddr,
        public_url: Option<&url::Url>,
        web_directory: Option<PathBuf>,
        update: Arc<demi_backend_database::update::Update>,
    ) -> io::Result<Listening> {
        let tcp = TcpListener::bind(address).await?.tap_io(listener::without_delay);
        let local_addr = tcp.local_addr()?;
        let public = demi_core_hosts::public_url::PublicUrl::default();
        public.listening(public_url, local_addr);
        let routes = Arc::new(std::sync::RwLock::new(update::routes(update, web_directory, public)));
        let closing = CancellationToken::new();
        let connections = CancellationToken::new();
        let stop = CancellationToken::new();
        let listener = EdgeListener::new(tcp, closing.clone(), connections.clone());
        let serving = {
            let routes = routes.clone();
            tokio::spawn(listener::serve(listener, stop.clone(), move |peer, request| {
                // Nothing panics while it holds the lock.
                let app = routes.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
                respond(app, peer, request)
            }))
        };
        Ok(Listening {
            local_addr,
            routes,
            closing,
            connections,
            stop,
            serving,
        })
    }
}

impl Listening {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Serves every route on the listener, the runner routes on
    /// `runner_socket` when there is one ([`runner_socket`]), and the
    /// management socket's requests on `manage_socket` when there is one
    /// ([`manage_socket`]).
    pub fn open(
        self,
        runner_socket: Option<RunnerSockets>,
        manage_socket: Option<UnixListener>,
        state: AppState,
        web_directory: Option<PathBuf>,
    ) -> Edge {
        let Self {
            local_addr,
            routes,
            closing,
            connections,
            stop,
            serving,
        } = self;
        // Before the first request: a Cloud's boot and a local store's
        // downloads name this URL.
        state
            .services
            .public_url
            .listening(state.site.public_url.as_ref(), local_addr);
        let mut serving_runners = Vec::new();
        if let Some(RunnerSockets { file, named }) = runner_socket {
            let app = runner_socket_router(state.clone(), closing.clone());
            // Only a gVisor sandbox reads the file's socket.
            let file = file.tap_io(listener::within_one_sandbox_read);
            let file = EdgeListener::new(file, closing.clone(), connections.clone());
            let served = app.clone();
            serving_runners.push(tokio::spawn(listener::serve(file, stop.clone(), move |peer, request| {
                respond(served.clone(), peer, request)
            })));
            if let Some(named) = named {
                let named = EdgeListener::new(named, closing.clone(), connections.clone());
                serving_runners.push(tokio::spawn(listener::serve(named, stop.clone(), move |peer, request| {
                    respond(app.clone(), peer, request)
                })));
            }
        }
        let serving_management = manage_socket
            .map(|socket| tokio::spawn(manage::serve(socket, state.clone(), stop.clone())));
        let app = router(state.clone(), closing.clone(), web_directory);
        *routes.write().unwrap_or_else(std::sync::PoisonError::into_inner) = app;
        Edge {
            local_addr,
            closing,
            connections,
            stop,
            serving,
            serving_runners,
            serving_management,
        }
    }

    /// Stops listening, for a start that failed before it served.
    pub async fn close(self) -> io::Result<()> {
        self.closing.cancel();
        self.connections.cancel();
        self.stop.cancel();
        self.serving.await.map_err(io::Error::other)
    }
}

impl Edge {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Closes the listener. Open connections go on serving, but a new
    /// request on one answers 503 `backend_closing`.
    pub fn stop_accepting(&self) {
        self.closing.cancel();
    }

    /// Closes the connections still open, such as a download's, and waits
    /// for the server to end.
    pub async fn close(self) -> io::Result<()> {
        self.closing.cancel();
        self.connections.cancel();
        self.stop.cancel();
        for serving in self.serving_runners {
            serving.await.map_err(io::Error::other)?;
        }
        if let Some(serving) = self.serving_management {
            serving.await.map_err(io::Error::other)?;
        }
        self.serving.await.map_err(io::Error::other)
    }
}

/// Answers one request with the routes, which see the connection it came
/// on.
async fn respond(app: Router, peer: Peer, mut request: Request<Incoming>) -> Response {
    request.extensions_mut().insert(ConnectInfo(peer));
    match app.oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    }
}

/// What the routes reach: the core's routes' own, the server's services, and
/// how the backend is reached from outside.
#[derive(Clone)]
pub struct AppState {
    /// What the users' cores' routes reach (`crates/core-api/src/api.rs`).
    pub api: Api,
    pub services: Arc<Services>,
    pub site: Arc<Site>,
    /// The links of the reply blocks shared pages frame.
    pub blocks: Arc<BlockLinks>,
    /// The update before this start, which `GET /api/update` says is over.
    pub update: Arc<demi_backend_database::update::Update>,
}

impl FromRef<AppState> for Api {
    fn from_ref(state: &AppState) -> Self {
        state.api.clone()
    }
}

impl FromRef<AppState> for Arc<demi_backend_database::update::Update> {
    fn from_ref(state: &AppState) -> Self {
        state.update.clone()
    }
}

impl FromRef<AppState> for Arc<Services> {
    fn from_ref(state: &AppState) -> Self {
        state.services.clone()
    }
}

impl FromRef<AppState> for Shards {
    fn from_ref(state: &AppState) -> Self {
        state.api.shards.clone()
    }
}

/// The routes.
///
/// The web app's routes, the entrances that sign a user in and every route the
/// session cookie authenticates, refuse a request that could act from a page
/// other than the product's before anything else.
///
/// Every `/api` path, unknown paths included, passes the session gate, except
/// those outside `session_api`:
///
/// - the entrances: setup, login, the passkey sign-in and spending a handoff
///   code;
/// - the shares' public routes;
/// - the routes that authenticate with device credentials instead;
/// - the files a page's frames load by a token or a plugin's name
///   (`core_api::public_routes`);
/// - the page connection, which checks the session without renewing it, and
///   the synchronization channel of a page of an earlier build.
///
/// The users' cores' routes (`core_api::routes`) sit beside the server's own
/// behind the gate, which gives each request its session as the caller.
///
/// The pipes stay outside the 503 of a closing backend, whose shutdown needs
/// them, and outside the body limit, since their bodies have none.
fn router(state: AppState, closing: CancellationToken, web_directory: Option<PathBuf>) -> Router {
    let session_api = Router::new()
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me).patch(auth::set_nickname))
        .route("/auth/password", put(auth::change_password))
        .route("/auth/email", post(auth::start_email_change))
        .route("/auth/email/confirm", post(auth::confirm_email_change))
        .route("/auth/passkeys", get(passkeys::list).post(passkeys::add))
        .route("/auth/passkeys/options", post(passkeys::add_options))
        .route("/auth/verify", post(sessions::verify))
        .route(
            "/auth/verify/passkey/options",
            post(sessions::verify_passkey_options),
        )
        .route("/auth/verify/passkey", post(sessions::verify_passkey))
        .route("/auth/handoff/code", post(sessions::make_code))
        .route("/auth/sessions", get(sessions::list))
        .route("/auth/sessions/{id}", delete(sessions::sign_out))
        .route(
            "/auth/passkeys/{id}",
            patch(passkeys::rename).delete(passkeys::remove),
        )
        .route("/state", get(state::read))
        .route("/settings", get(settings::settings))
        .route("/settings/instance-mode", put(settings::set_instance_mode))
        .route(
            "/settings/cloud-network",
            get(settings::cloud_network).put(settings::set_cloud_network),
        )
        .route(
            "/settings/cloud-isolation",
            get(settings::cloud_isolation).put(settings::set_cloud_isolation),
        )
        .route("/providers/{id}/share-keys", get(lending::list).post(lending::create))
        .route("/providers/{id}/share-keys/{key}", delete(lending::revoke))
        .route("/server-upgrade", get(server_upgrade::read))
        .route("/server-upgrade/channel", put(server_upgrade::set_channel))
        .route("/server-upgrade/prepare-ahead", put(server_upgrade::set_prepare_ahead))
        .route("/server-upgrade/check", post(server_upgrade::check))
        .route("/server-upgrade/run", post(server_upgrade::run))
        .route("/server-upgrade/agents", get(server_upgrade::agents))
        .route("/users", get(users::list).post(users::create))
        .route("/users/{id}", patch(users::change))
        .route("/users/{id}/password", post(users::new_first_password))
        .route("/users/{id}/sign-out", post(users::sign_out))
        .route("/users/{id}/sign-in-code", post(users::sign_in_code))
        .route("/usage/instance", get(usage::instance))
        .route("/log", post(diagnostics::log))
        .route("/conversations/{id}/shares", get(shares::list).post(shares::create))
        .route(
            "/conversations/{id}/shares/{token}",
            put(shares::update).delete(shares::remove),
        )
        .route("/push/key", get(push::key))
        .route(
            "/push/subscription",
            put(push::subscribe).delete(push::unsubscribe),
        )
        .route(
            "/conversations/{id}/notifications-card/dismiss",
            post(push::dismiss_card),
        )
        .route("/cloud", get(cloud::status))
        .route("/cloud/reset", post(cloud::reset))
        .route("/devices/claim", post(pairing::claim))
        .merge(demi_core_api::routes().with_state(state.api.clone()))
        // The reads' caching (`revalidate.rs`): the routes' answers only.
        .layer(middleware::from_fn(demi_core_api::revalidate::reads))
        .fallback(no_route)
        .method_not_allowed_fallback(no_route)
        // After the fallbacks, so the gate covers them too.
        .layer(middleware::from_fn_with_state(
            state.services.clone(),
            gate::session,
        ))
        .merge(demi_core_api::public_routes().with_state(state.api.clone()));
    let web_app_routes = Router::new()
        .route("/api/setup", get(auth::setup_status).post(auth::setup))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/passkey/options", post(passkeys::sign_in_options))
        .route("/api/auth/passkey", post(passkeys::sign_in))
        .route("/api/auth/handoff", post(sessions::spend_code))
        .route(
            &format!("/api{CONNECT_ROUTE}"),
            get(connection::connect)
                .layer(Extension(Closing(closing.clone())))
                .layer(middleware::from_fn_with_state(state.services.clone(), gate::page_connection)),
        )
        .route("/api/sync", get(demi_core_api::sync::legacy))
        .merge(public_shares())
        .method_not_allowed_fallback(no_route)
        .nest("/api", session_api)
        // After the nest, so the check covers every web app route, the
        // unknown paths under `/api` included.
        .layer(middleware::from_fn_with_state(
            state.site.clone(),
            gate::product_pages,
        ));
    let app = Router::new()
        .merge(runners_and_downloads())
        .merge(hooks())
        .merge(relay())
        .merge(web_app_routes);
    let app = match web_directory {
        Some(directory) => assets::serve(app, directory, state.services.public_url.clone()),
        None => app.fallback(no_route),
    };
    edge(app, closing, state)
}

/// The public routes of the shares: no session gate, the token their only key, each
/// answer with the headers that keep the token and the snapshot out of indexes,
/// referrers and caches.
fn public_shares() -> Router<AppState> {
    Router::new()
        .route("/api/shares/{token}", get(shares::read))
        .route("/api/shares/{token}/transcript", get(shares::transcript))
        .route(
            "/api/shares/{token}/transcript/blocks/{block}",
            get(shares::block),
        )
        .route("/api/shares/{token}/blobs/{sha256}", get(shares::blob))
        .route(
            "/api/shares/{token}/plugins/{plugin}/blocks/{tag}/{id}",
            get(shared_blocks::shared_document),
        )
        .route("/api/blocks/{link}", get(shared_blocks::linked_document))
        .route(
            "/api/shares/{token}/attachments/{attachment}",
            get(shares::attachment),
        )
        .layer(middleware::map_response(shares::public_headers))
}

/// The schedules' URLs: no session gate and no check of the page they come
/// from, since another system calls them; the id and the secret are their only
/// keys.
fn hooks() -> Router<AppState> {
    Router::new().route("/hooks/{id}", axum::routing::any(demi_core_api::schedules::hook))
}

/// What a server that borrows an entry of this one's calls: no session gate
/// and no check of the page it comes from, since another server calls it; the
/// share key is its only key.
fn relay() -> Router<AppState> {
    use demi_provider_relay::wire::media::MEDIA_ROUTE;
    // A media call's body, an edit's or a video's carrying their images, is
    // read within the limit of one frame, as `lending::media` reads it.
    let calls = DefaultBodyLimit::max(demi_provider_relay::wire::MESSAGE_BYTES + 4);
    Router::new()
        .route(demi_provider_relay::wire::PROVIDER_ROUTE, get(lending::provider))
        .route(demi_provider_relay::wire::SOCKET_ROUTE, get(lending::socket))
        .route(&format!("{MEDIA_ROUTE}/{{kind}}"), post(lending::media).layer(calls))
}

/// The routes of the backend's runner socket: what a Cloud's runner reaches
/// its backend for, and nothing a browser uses.
fn runner_socket_router(state: AppState, closing: CancellationToken) -> Router {
    edge(runners_and_downloads().fallback(no_route), closing, state)
}

/// What no session cookie authenticates: the public downloads, and the
/// runner's socket, which carries a device token.
fn runners_and_downloads() -> Router<AppState> {
    Router::new()
        .route("/install.sh", get(install::shell))
        .route("/install.ps1", get(install::powershell))
        .route(
            "/runner-artifacts/{release}/{target}/{file}",
            get(install::artifact),
        )
        .route(
            &format!("{}/{{sha256}}", native::ROUTE),
            get(install::native_artifact),
        )
        .route("/api/runner", get(runners::socket))
        .route("/api/update", get(update::read))
        .method_not_allowed_fallback(no_route)
}

/// `app` as a listener serves it: with the body limit and the 503 of a
/// closing backend, and the pipes beside both.
fn edge(app: Router<AppState>, closing: CancellationToken, state: AppState) -> Router {
    let pipes = demi_core_api::runners::routes()
        .method_not_allowed_fallback(no_route)
        .with_state(state.api.clone())
        .layer(middleware::from_fn_with_state(state.clone(), runners::token_device));
    app.layer(DefaultBodyLimit::max(demi_core_api::body::JSON_BODY_LIMIT))
        .layer(middleware::from_fn_with_state(
            closing,
            refuse_while_closing,
        ))
        .merge(pipes)
        // Inside the request's span, so an abandoned request's line carries
        // its fields.
        .layer(middleware::from_fn(demi_core_api::request_id::watch_abandon))
        // The request's line is `request_id::answered`'s alone: the layer's
        // own failure line would say a 5xx answer twice.
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::http::Request<axum::body::Body>| {
                    demi_core_api::request_id::span(request)
                })
                .on_response(demi_core_api::request_id::answered)
                .on_failure(()),
        )
        // Outermost, so every answer carries the request's ID, a refusal of
        // the layers inside included.
        .layer(middleware::from_fn(demi_core_api::request_id::assign))
        .with_state(state)
}

/// The 500 of an error of the server's own services, such as its password
/// hashing, which its message describes.
pub(crate) fn internal(error: impl std::error::Error + 'static) -> ApiError {
    ApiError::internal(&error)
}

/// The JSON 404 of a method and path no route answers.
async fn no_route(method: Method, OriginalUri(uri): OriginalUri) -> ApiError {
    ApiError::no_route(&method, uri.path())
}

/// Answers 503 `backend_closing` once shutdown starts. Shutdown itself needs
/// the runners' pipes, so the routes that carry them stay outside this
/// layer.
async fn refuse_while_closing(
    State(closing): State<CancellationToken>,
    request: Request,
    next: Next,
) -> Response {
    if closing.is_cancelled() {
        return ApiError::backend_closing().into_response();
    }
    next.run(request).await
}
