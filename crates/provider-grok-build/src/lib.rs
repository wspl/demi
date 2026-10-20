//! The Grok Build provider: a `grok-build` subscription entry's account, signed
//! in by OAuth device authorization at `auth.x.ai`, inferring over Chat
//! Completions through the chat proxy Grok's own CLI uses.
//!
//! The provider:
//!
//! - refreshes the account's OIDC tokens by the one protocol of
//!   `crates/provider-grok-build/src/auth.rs`;
//! - reads the model catalog from `/v1/models`;
//! - keeps the account's quota from a free billing and subscription probe and
//!   the rate-limit headers of every chat response;
//! - makes images and videos through the proxy's image and video endpoints.

mod auth;
mod images;
mod login;
mod models;
mod quota;
mod releases;
mod request;
mod run;
mod videos;

pub use releases::GrokReleases;

use std::sync::Arc;

use demi_provider_common::{
    Capabilities, CatalogError, InferenceRequest, Provider, ProviderRun, ProviderRuntime,
    RequestLimits, ToolResultKinds, RuntimeEnv, RuntimeError,
    credentials::{Accounts, CredentialPool, SubscriptionAccounts},
    images::ImageMaker,
    videos::VideoMaker,
    endpoint_url,
    quota::{ProviderQuota, QuotaSnapshotStore},
    read_http_failure,
};
use demi_shared_types::{
    AuthState, Clock, Model, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList,
    RuntimeState, Timestamp,
};
use futures_util::{
    StreamExt,
    future::{BoxFuture, LocalBoxFuture},
};
use reqwest::Url;

use crate::{auth::GrokAuth, login::GrokKit, quota::GrokQuota};

/// How failure messages name the vendor.
const LABEL: &str = "Grok Build";

/// The configuration of a `grok-build` entry's provider for one account.
#[derive(Debug, Clone)]
pub struct GrokConfig {
    /// The account the provider stands for; `None` only for a provider built
    /// to log in, which has no account yet.
    pub account: Option<String>,
    /// The chat proxy, `https://cli-chat-proxy.grok.com/v1` in the product.
    pub proxy_url: Url,
    /// The issuer device logins sign in at, `https://auth.x.ai` in the
    /// product. A signed-in account refreshes at the issuer it names.
    pub issuer_url: Url,
    /// The newest stable Grok CLI release, the backend's one.
    pub releases: GrokReleases,
}

impl GrokConfig {
    pub const PROXY_URL: &str = "https://cli-chat-proxy.grok.com/v1";
    pub const ISSUER_URL: &str = "https://auth.x.ai";

    /// The product's configuration of an entry's provider for `account`,
    /// whose requests name the version `releases` knows.
    pub fn new(account: Option<String>, releases: GrokReleases) -> Self {
        Self {
            account,
            proxy_url: Url::parse(Self::PROXY_URL).expect("the proxy URL parses"),
            issuer_url: Url::parse(Self::ISSUER_URL).expect("the issuer URL parses"),
            releases,
        }
    }
}

/// A `grok-build` entry's provider for one account, shared by every user and
/// request of the entry.
pub struct GrokProvider {
    shared: Arc<Shared>,
    images: images::GrokImages,
    videos: videos::GrokVideos,
}

/// What the provider and all its runtimes share.
struct Shared {
    chat_url: Url,
    models_url: Url,
    image_generations_url: Url,
    image_edits_url: Url,
    video_generations_url: Url,
    /// The proxy's base, under which a video job's state is read.
    proxy_url: Url,
    auth: Arc<GrokAuth>,
    quota: ProviderQuota,
    accounts: Accounts<GrokKit>,
    /// The Grok CLI release every request names.
    releases: GrokReleases,
    /// The client of the provider's own requests: catalogs, probes,
    /// refreshes outside a run and logins.
    http: demi_provider_common::ProviderHttp,
    clock: Arc<dyn Clock>,
}

impl GrokProvider {
    /// A provider over the entry's `pool`, keeping the account's quota in
    /// `snapshots`; `http` is the client of the edge that owns it.
    pub fn new(
        config: GrokConfig,
        pool: Arc<dyn CredentialPool>,
        snapshots: Arc<dyn QuotaSnapshotStore>,
        http: demi_provider_common::ProviderHttp,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let auth = Arc::new(GrokAuth::new(pool.clone(), config.account, clock.clone()));
        let mut user_url = endpoint_url(&config.proxy_url, "/user");
        let mut billing_url = endpoint_url(&config.proxy_url, "/billing");
        let kit = GrokKit {
            http: http.clone(),
            issuer: config.issuer_url,
            user_url: user_url.clone(),
            releases: config.releases.clone(),
            clock: clock.clone(),
        };
        user_url.set_query(Some("include=subscription"));
        billing_url.set_query(Some("format=credits"));
        let source = GrokQuota {
            auth: auth.clone(),
            http: http.clone(),
            releases: config.releases.clone(),
            user_url,
            billing_url,
        };
        let shared = Arc::new(Shared {
                chat_url: endpoint_url(&config.proxy_url, "/chat/completions"),
                models_url: endpoint_url(&config.proxy_url, "/models"),
                image_generations_url: endpoint_url(&config.proxy_url, "/images/generations"),
                image_edits_url: endpoint_url(&config.proxy_url, "/images/edits"),
                video_generations_url: endpoint_url(&config.proxy_url, "/videos/generations"),
                proxy_url: config.proxy_url.clone(),
                auth,
                quota: ProviderQuota::new(Box::new(source), snapshots, clock.clone()),
                accounts: Accounts::new(pool, kit, clock.clone()),
                releases: config.releases,
                http,
                clock,
            });
        Self {
            images: images::GrokImages(shared.clone()),
            videos: videos::GrokVideos(shared.clone()),
            shared,
        }
    }
}

impl Provider for GrokProvider {
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
            message: Some("Uses the Grok Build chat proxy with the account's sign-in".into()),
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
        read_http_failure(diagnostics, received_at)
    }

    fn quota(&self) -> Option<&ProviderQuota> {
        Some(&self.shared.quota)
    }

    fn accounts(&self) -> Option<&dyn SubscriptionAccounts> {
        Some(&self.shared.accounts)
    }

    fn images(&self) -> Option<&dyn ImageMaker> {
        Some(&self.images)
    }

    fn videos(&self) -> Option<&dyn VideoMaker> {
        Some(&self.videos)
    }

    fn runtime(&self, env: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        Ok(Box::new(GrokRuntime {
            shared: self.shared.clone(),
            http: env.http,
        }))
    }
}

/// A session's runtime: it holds nothing between runs.
struct GrokRuntime {
    shared: Arc<Shared>,
    http: demi_provider_common::ProviderHttp,
}

impl ProviderRuntime for GrokRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        run::run(self.shared.clone(), self.http.clone(), request).boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(GrokRuntime {
            shared: self.shared.clone(),
            http: self.http.clone(),
        })
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {})
    }

    /// The Grok Build proxy documents none.
    /// The proxy takes no media in a tool message and documents no
    /// follow-up, so a tool result carries none (`crates/provider-common/src/contract.rs`).
    fn request_limits(&self, _model: &Model) -> RequestLimits {
        RequestLimits {
            tool_results: ToolResultKinds::NONE,
            ..RequestLimits::default()
        }
    }
}

/// Test support (feature `testing`): the body this provider sends for a
/// request, for the tests of what sends its requests.
#[cfg(feature = "testing")]
pub mod testing {
    use demi_provider_common::InferenceRequest;

    pub fn body(request: &InferenceRequest) -> Vec<u8> {
        crate::request::encode(request)
    }
}
