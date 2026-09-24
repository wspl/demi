//! The Claude Code provider (`crates-and-packages.md` §
//! provider-claude-code): a `claude-code` subscription entry's account, a
//! setup token, inferring through Demi's own copy of the vendor's CLI. A
//! session's runtime keeps one CLI process and exchanges stream-json with it
//! over the `shell` process a [`Placement`] starts; the model's tools reach
//! Demi over the SDK MCP channel, an rmcp server inside the run, with the
//! model's parallel tool batches kept whole. The catalog is models.dev's
//! Claude models, and the account's quota comes from a free probe of the OAuth
//! usage endpoint and from the rate limits the CLI's lines report
//! (`claude-code.md`).

mod account;
mod cli;
mod input;
mod live;
mod mcp;
mod models;
mod output;
mod placement;
mod quota;
mod run;

use std::rc::Rc;
use std::sync::Arc;

use demi_core::{
    AuthState, Clock, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList,
    RuntimeState, Timestamp,
};
use demi_provider::credentials::{Accounts, CredentialPool, SubscriptionAccounts};
use demi_provider::models_dev::ModelsDevClient;
use demi_provider::quota::{ProviderQuota, QuotaSnapshotStore};
use demi_provider::{
    Capabilities, CatalogError, Provider, ProviderRuntime, RuntimeEnv, RuntimeError,
    read_http_failure,
};
use futures_util::future::BoxFuture;
use reqwest::Url;

pub use placement::{CliSite, Placement, StartError};

use crate::account::{ClaudeAuth, ClaudeKit};
use crate::quota::ClaudeQuota;
use crate::run::ClaudeCodeRuntime;

/// How messages name the family.
const FAMILY: &str = "Claude Code";

/// The configuration of a `claude-code` entry's provider for one account.
#[derive(Debug, Clone)]
pub struct ClaudeCodeConfig {
    /// The entry's id.
    pub id: String,
    /// The entry's label.
    pub display_name: String,
    /// The account the provider stands for; `None` only for a provider built
    /// to add the entry's first account.
    pub account: Option<String>,
    /// The OAuth usage endpoint the quota probe reads,
    /// `https://api.anthropic.com/api/oauth/usage` in the product.
    pub usage_url: Url,
}

impl ClaudeCodeConfig {
    pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

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
            usage_url: Url::parse(Self::USAGE_URL).expect("the usage URL parses"),
        }
    }
}

/// A `claude-code` entry's provider for one account, shared by every user
/// and request of the entry. Its runtimes start their CLI through a
/// placement ([`ClaudeCodeProvider::process_runtime`]); the generic
/// [`Provider::runtime`] refuses, since only the backend can choose the
/// machine (`claude-code.md` § How a runtime gets its process).
pub struct ClaudeCodeProvider {
    shared: Arc<Shared>,
}

/// What the provider and all its runtimes share.
struct Shared {
    id: String,
    display_name: String,
    auth: Arc<ClaudeAuth>,
    quota: ProviderQuota,
    accounts: Accounts<ClaudeKit>,
    models_dev: ModelsDevClient,
}

impl ClaudeCodeProvider {
    /// A provider over the entry's `pool`, keeping the account's quota in
    /// `snapshots`; `models_dev` is the backend's one models.dev copy, and
    /// `http` the client of the edge that owns the provider, for its quota
    /// probes.
    pub fn new(
        config: ClaudeCodeConfig,
        pool: Arc<dyn CredentialPool>,
        snapshots: Arc<dyn QuotaSnapshotStore>,
        models_dev: ModelsDevClient,
        http: reqwest::Client,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let auth = Arc::new(ClaudeAuth::new(pool.clone(), config.account));
        let source = ClaudeQuota {
            auth: auth.clone(),
            http,
            usage_url: config.usage_url,
        };
        Self {
            shared: Arc::new(Shared {
                id: config.id,
                display_name: config.display_name,
                auth,
                quota: ProviderQuota::new(Box::new(source), snapshots, clock.clone()),
                accounts: Accounts::new(pool, ClaudeKit, clock),
                models_dev,
            }),
        }
    }

    /// A session's runtime, which starts its CLI processes through
    /// `placement` and keeps at most one of them between runs.
    pub fn process_runtime(&self, placement: Rc<dyn Placement>) -> Box<dyn ProviderRuntime> {
        Box::new(ClaudeCodeRuntime::new(self.shared.clone(), placement))
    }
}

impl Provider for ClaudeCodeProvider {
    fn id(&self) -> &str {
        &self.shared.id
    }

    fn display_name(&self) -> &str {
        &self.shared.display_name
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities { process_host: true }
    }

    /// The account's setup token as stored; reading it never infers.
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

    /// Whether the CLI runs is known only when a request starts it on the
    /// user's Cloud.
    fn runtime_state(&self) -> RuntimeState {
        RuntimeState::Unknown {
            message: Some("Claude Code runs on the user's Cloud when a request needs it".into()),
        }
    }

    fn list_models(&self) -> BoxFuture<'_, Result<ProviderModelList, CatalogError>> {
        Box::pin(models::list(&self.shared.models_dev))
    }

    /// The CLI's failures carry no HTTP record of the vendor's; a record
    /// that has one is read by the standard HTTP rules.
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

    fn runtime(&self, _env: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        Err(RuntimeError::ProcessHostRequired {
            provider: self.shared.display_name.clone(),
        })
    }
}
