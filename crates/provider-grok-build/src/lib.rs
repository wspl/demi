//! The Grok Build provider (`crates-and-packages.md` § Vendor provider
//! crates): a `grok-build` subscription entry's account, signed in by OAuth
//! device authorization at `auth.x.ai`, inferring over Chat Completions
//! through the chat proxy Grok's own CLI uses. It refreshes the account's
//! OIDC tokens by the one protocol of `providers.md` § Token refresh, reads
//! the model catalog from `/v1/models`, and keeps the account's quota from a
//! free billing and subscription probe and the rate-limit headers of every
//! chat response.

mod auth;
mod login;
mod models;
mod quota;
mod request;
mod run;

use std::sync::Arc;

use demi_core::{
    AuthState, Clock, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList,
    RuntimeState, Timestamp,
};
use demi_provider::{
    Capabilities, CatalogError, InferenceRequest, Provider, ProviderRun, ProviderRuntime,
    RuntimeEnv, RuntimeError,
    credentials::{Accounts, CredentialPool, SubscriptionAccounts},
    endpoint_url,
    quota::{ProviderQuota, QuotaSnapshotStore},
    read_http_failure,
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
    /// The entry's id.
    pub id: String,
    /// The entry's label.
    pub display_name: String,
    /// The account the provider stands for; `None` only for a provider built
    /// to log in, which has no account yet.
    pub account: Option<String>,
    /// The chat proxy, `https://cli-chat-proxy.grok.com/v1` in the product.
    pub proxy_url: Url,
    /// The issuer device logins sign in at, `https://auth.x.ai` in the
    /// product. A signed-in account refreshes at the issuer it names.
    pub issuer_url: Url,
}

impl GrokConfig {
    pub const PROXY_URL: &str = "https://cli-chat-proxy.grok.com/v1";
    pub const ISSUER_URL: &str = "https://auth.x.ai";

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
            proxy_url: Url::parse(Self::PROXY_URL).expect("the proxy URL parses"),
            issuer_url: Url::parse(Self::ISSUER_URL).expect("the issuer URL parses"),
        }
    }
}

/// A `grok-build` entry's provider for one account, shared by every user and
/// request of the entry.
pub struct GrokProvider {
    shared: Arc<Shared>,
}

/// What the provider and all its runtimes share.
struct Shared {
    id: String,
    display_name: String,
    chat_url: Url,
    models_url: Url,
    auth: Arc<GrokAuth>,
    quota: ProviderQuota,
    accounts: Accounts<GrokKit>,
    /// The client of the provider's own requests: catalogs, probes,
    /// refreshes outside a run and logins.
    http: reqwest::Client,
    clock: Arc<dyn Clock>,
}

impl GrokProvider {
    /// A provider over the entry's `pool`, keeping the account's quota in
    /// `snapshots`; `http` is the client of the edge that owns it.
    pub fn new(
        config: GrokConfig,
        pool: Arc<dyn CredentialPool>,
        snapshots: Arc<dyn QuotaSnapshotStore>,
        http: reqwest::Client,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let auth = Arc::new(GrokAuth::new(pool.clone(), config.account, clock.clone()));
        let mut user_url = endpoint_url(&config.proxy_url, "/user");
        let mut billing_url = endpoint_url(&config.proxy_url, "/billing");
        let kit = GrokKit {
            http: http.clone(),
            issuer: config.issuer_url,
            user_url: user_url.clone(),
            clock: clock.clone(),
        };
        user_url.set_query(Some("include=subscription"));
        billing_url.set_query(Some("format=credits"));
        let source = GrokQuota {
            auth: auth.clone(),
            http: http.clone(),
            user_url,
            billing_url,
        };
        Self {
            shared: Arc::new(Shared {
                id: config.id,
                display_name: config.display_name,
                chat_url: endpoint_url(&config.proxy_url, "/chat/completions"),
                models_url: endpoint_url(&config.proxy_url, "/models"),
                auth,
                quota: ProviderQuota::new(Box::new(source), snapshots, clock.clone()),
                accounts: Accounts::new(pool, kit, clock.clone()),
                http,
                clock,
            }),
        }
    }
}

impl Provider for GrokProvider {
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
    http: reqwest::Client,
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
}
