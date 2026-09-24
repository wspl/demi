//! The Codex provider (`crates-and-packages.md` § Vendor provider crates): a
//! `codex` subscription entry's account, signed in by Codex's device login,
//! inferring through the Codex Responses backend over a WebSocket with
//! server-sent events as the fallback. It refreshes the account's tokens by
//! the one protocol of `providers.md` § Token refresh, reads its failure
//! records' usage-limit resets, keeps the account's quota from the
//! `x-codex-*` headers and a free usage probe, and reads the account's model
//! catalog.

mod auth;
mod failure;
mod login;
mod models;
mod quota;
mod request;
mod run;
mod transport;

use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use demi_core::{
    AuthState, Clock, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList,
    RuntimeState, Timestamp,
};
use demi_provider::{
    Capabilities, CatalogError, InferenceRequest, Provider, ProviderRun, ProviderRuntime,
    RuntimeEnv, RuntimeError,
    credentials::{Accounts, CredentialPool, SubscriptionAccounts},
    quota::{ProviderQuota, QuotaSnapshotStore},
};
use futures_util::{
    StreamExt,
    future::{BoxFuture, LocalBoxFuture},
};
use http::HeaderValue;
use reqwest::Url;

use crate::{auth::CodexAuth, login::CodexKit, quota::CodexQuota};

pub use failure::read_codex_failure;

/// How failure messages name the vendor.
const LABEL: &str = "Codex";

/// The prefix this provider puts on the reasoning items it receives, so that
/// on replay it sends back only its vendor's.
const SIGNATURE_TAG: &str = "codex:";

/// How a request reaches the Responses backend.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TransportMode {
    /// A WebSocket first, server-sent events when it fails before its first
    /// event.
    #[default]
    Auto,
    Sse,
    WebSocket,
}

/// The configuration of a `codex` entry's provider for one account.
#[derive(Debug, Clone)]
pub struct CodexConfig {
    /// The entry's id.
    pub id: String,
    /// The entry's label.
    pub display_name: String,
    /// The account the provider stands for; `None` only for a provider built
    /// to log in, which has no account yet.
    pub account: Option<String>,
    /// The ChatGPT backend, `https://chatgpt.com/backend-api` in the product.
    pub backend_url: Url,
    /// The sign-in service, `https://auth.openai.com` in the product.
    pub auth_url: Url,
    pub transport: TransportMode,
    /// How long a server-sent events request waits for its response headers.
    pub header_timeout: Duration,
    /// How long a WebSocket waits to connect.
    pub connect_timeout: Duration,
    /// How long a WebSocket may go without a message; `None` for no limit.
    pub stream_idle_timeout: Option<Duration>,
}

impl CodexConfig {
    pub const BACKEND_URL: &str = "https://chatgpt.com/backend-api";
    pub const AUTH_URL: &str = "https://auth.openai.com";

    /// The product's configuration of an entry's provider for `account`.
    pub fn new(
        id: impl Into<String>,
        display_name: impl Into<String>,
        account: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
            account,
            backend_url: Url::parse(Self::BACKEND_URL).expect("the backend URL parses"),
            auth_url: Url::parse(Self::AUTH_URL).expect("the sign-in URL parses"),
            transport: TransportMode::Auto,
            header_timeout: Duration::from_secs(20),
            connect_timeout: Duration::from_secs(10),
            stream_idle_timeout: None,
        }
    }
}

/// A `codex` entry's provider for one account, shared by every user and
/// request of the entry.
pub struct CodexProvider {
    shared: Arc<Shared>,
}

/// What the provider and all its runtimes share.
struct Shared {
    id: String,
    display_name: String,
    responses_url: Url,
    websocket_url: Url,
    models_url: Url,
    transport: TransportMode,
    header_timeout: Duration,
    connect_timeout: Duration,
    stream_idle_timeout: Option<Duration>,
    /// Set once a WebSocket could not connect at all: the provider's later
    /// requests go over server-sent events at once.
    websocket_unreachable: AtomicBool,
    user_agent: HeaderValue,
    auth: Arc<CodexAuth>,
    quota: ProviderQuota,
    accounts: Accounts<CodexKit>,
    /// The client of the provider's own requests: catalogs, probes,
    /// refreshes outside a run and logins.
    http: reqwest::Client,
    clock: Arc<dyn Clock>,
}

impl CodexProvider {
    /// A provider over the entry's `pool`, keeping the account's quota in
    /// `snapshots`; `http` is the client of the edge that owns it.
    pub fn new(
        config: CodexConfig,
        pool: Arc<dyn CredentialPool>,
        snapshots: Arc<dyn QuotaSnapshotStore>,
        http: reqwest::Client,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let user_agent = format!(
            "demi-codex-provider/{} ({}; {})",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        // A crate version and a platform's names are header text.
        let user_agent =
            HeaderValue::from_str(&user_agent).expect("the user agent is a header value");
        let auth = Arc::new(CodexAuth::new(
            pool.clone(),
            config.account,
            &config.auth_url,
            clock.clone(),
        ));
        let source = CodexQuota {
            auth: auth.clone(),
            http: http.clone(),
            usage_url: demi_provider::endpoint_url(&config.backend_url, "/wham/usage"),
            user_agent: user_agent.clone(),
        };
        let kit = CodexKit {
            http: http.clone(),
            auth_url: config.auth_url.clone(),
            clock: clock.clone(),
        };
        let responses_url = codex_url(&config.backend_url, "/responses");
        Self {
            shared: Arc::new(Shared {
                id: config.id,
                display_name: config.display_name,
                websocket_url: websocket_url(&responses_url),
                responses_url,
                models_url: models::models_url(&config.backend_url),
                transport: config.transport,
                header_timeout: config.header_timeout,
                connect_timeout: config.connect_timeout,
                stream_idle_timeout: config.stream_idle_timeout,
                websocket_unreachable: AtomicBool::new(false),
                user_agent,
                auth,
                quota: ProviderQuota::new(Box::new(source), snapshots, clock.clone()),
                accounts: Accounts::new(pool, kit, clock.clone()),
                http,
                clock,
            }),
        }
    }
}

/// `path` under the backend's `/codex`, unless the base already names it.
fn codex_url(backend: &Url, path: &str) -> Url {
    let trimmed = backend.path().trim_end_matches('/');
    if trimmed.ends_with(path) {
        return demi_provider::endpoint_url(backend, path);
    }
    let codex = if trimmed.ends_with("/codex") {
        backend.clone()
    } else {
        demi_provider::endpoint_url(backend, "/codex")
    };
    demi_provider::endpoint_url(&codex, path)
}

/// The WebSocket address of a Responses URL: `wss` for `https`, `ws` for
/// `http`.
fn websocket_url(responses: &Url) -> Url {
    let mut url = responses.clone();
    let scheme = if url.scheme() == "http" { "ws" } else { "wss" };
    // http and https URLs take either WebSocket scheme.
    url.set_scheme(scheme)
        .expect("a WebSocket scheme is valid for an HTTP URL");
    url
}

impl Provider for CodexProvider {
    fn id(&self) -> &str {
        &self.shared.id
    }

    fn display_name(&self) -> &str {
        &self.shared.display_name
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    /// The account's sign-in as stored; reading it never refreshes and never
    /// infers.
    fn auth_status(&self) -> BoxFuture<'_, AuthState> {
        Box::pin(async {
            match self.shared.auth.stored().await {
                Ok(secret) => AuthState::Authenticated {
                    account_label: Some(secret.label().label),
                },
                Err(failure) => failure.state(),
            }
        })
    }

    fn runtime_state(&self) -> RuntimeState {
        RuntimeState::Ready {
            message: Some("Uses the Codex backend with the account's ChatGPT sign-in".into()),
        }
    }

    fn list_models(&self) -> BoxFuture<'_, Result<ProviderModelList, CatalogError>> {
        Box::pin(models::list(&self.shared))
    }

    fn read_failure(
        &self,
        diagnostics: &ProviderErrorDiagnostics,
        received_at: Timestamp,
    ) -> ProviderFailureFacts {
        read_codex_failure(diagnostics, received_at)
    }

    fn quota(&self) -> Option<&ProviderQuota> {
        Some(&self.shared.quota)
    }

    fn accounts(&self) -> Option<&dyn SubscriptionAccounts> {
        Some(&self.shared.accounts)
    }

    fn runtime(&self, env: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        Ok(Box::new(CodexRuntime {
            shared: self.shared.clone(),
            http: env.http,
        }))
    }
}

/// A session's runtime: it holds nothing between runs.
struct CodexRuntime {
    shared: Arc<Shared>,
    http: reqwest::Client,
}

impl ProviderRuntime for CodexRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        run::run(self.shared.clone(), self.http.clone(), request).boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(CodexRuntime {
            shared: self.shared.clone(),
            http: self.http.clone(),
        })
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {})
    }
}
