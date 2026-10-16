//! The Claude Code provider (`crates-and-packages.md` §
//! provider-claude-code): a `claude-code` subscription entry's account,
//! signed in by the CLI's own login on the Claude Code machine, inferring
//! through Demi's own copy of the vendor's CLI. A session's runtime keeps
//! one CLI process and exchanges stream-json with it over the `shell`
//! process a [`Placement`] starts; a new process resumes a session written
//! from the conversation's blocks, whose entries the CLI mirrors back onto
//! them; every process of an account runs in the account's one directory,
//! where the CLI keeps and renews the sign-in itself; the model's tools
//! reach Demi over the SDK MCP channel, an rmcp server inside the run, with
//! the model's parallel tool batches kept whole. The catalog is models.dev's
//! Claude models, and the account's quota comes from the CLI's own usage
//! answer and from the rate limits its lines report (`claude-code.md`).

mod account;
mod cli;
mod failure;
mod files;
mod input;
mod live;
mod login;
mod mcp;
mod models;
mod output;
mod placement;
mod prepared;
mod mirror;
mod quota;
mod run;
mod session;
mod timing;

use std::rc::Rc;
use std::sync::Arc;

use demi_provider_common::credentials::{Accounts, CredentialPool, SubscriptionAccounts};
use demi_provider_common::models_dev::ModelsDevClient;
use demi_provider_common::quota::{ProviderQuota, QuotaSnapshotStore};
use demi_provider_common::{
    Capabilities, CatalogError, Provider, ProviderRuntime, RuntimeEnv, RuntimeError,
};
use demi_shared_types::{
    AuthState, Clock, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList,
    RuntimeState, Timestamp,
};
use futures_util::future::BoxFuture;

pub use placement::{
    AccountMachine, AccountWork, CliSite, CliStart, CliStarter, CliSystem, CliWork, ConfigDir,
    Handover, Placed, Placement, StartError, StartStage, StartSteps, WorkLease,
};
pub use live::close_process;
pub use prepared::PreparedProcesses;

use crate::account::ClaudeAuth;
use crate::login::ClaudeKit;
use crate::quota::ClaudeQuota;
use crate::run::ClaudeCodeRuntime;

/// How messages name the family.
const FAMILY: &str = "Claude Code";

/// The configuration of a `claude-code` entry's provider for one account.
#[derive(Clone)]
pub struct ClaudeCodeConfig {
    /// The entry's id, which names the entry in the logs of its runs.
    pub id: String,
    /// The entry's label, which names the provider in its refusals.
    pub display_name: String,
    /// The account the provider stands for; `None` only for a provider built
    /// to add the entry's first account.
    pub account: Option<String>,
    /// The Claude Code machine, which the account's usage probe runs on;
    /// none for a provider that cannot reach it, which cannot probe.
    pub machine: Option<Arc<dyn AccountMachine>>,
}

impl ClaudeCodeConfig {
    /// The configuration of an entry's provider for `account`.
    pub fn new(
        id: impl Into<String>,
        display_name: impl Into<String>,
        account: Option<String>,
        machine: Option<Arc<dyn AccountMachine>>,
    ) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
            account,
            machine,
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
    pool: Arc<dyn CredentialPool>,
    clock: Arc<dyn Clock>,
    auth: Arc<ClaudeAuth>,
    quota: ProviderQuota,
    /// The accounts as inference and settings see them; signing in needs a
    /// machine ([`ClaudeCodeProvider::login_accounts`]).
    accounts: Accounts<ClaudeKit>,
    models_dev: ModelsDevClient,
}

impl ClaudeCodeProvider {
    /// A provider over the entry's `pool`, keeping the account's quota in
    /// `snapshots`; `models_dev` is the backend's one models.dev copy.
    pub fn new(
        config: ClaudeCodeConfig,
        pool: Arc<dyn CredentialPool>,
        snapshots: Arc<dyn QuotaSnapshotStore>,
        models_dev: ModelsDevClient,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let auth = Arc::new(ClaudeAuth::new(pool.clone(), config.account));
        let source = ClaudeQuota {
            auth: auth.clone(),
            machine: config.machine,
            entry: config.id.clone(),
        };
        let kit = ClaudeKit {
            machine: None,
            entry: config.id.clone(),
            pool: pool.clone(),
        };
        Self {
            shared: Arc::new(Shared {
                id: config.id,
                display_name: config.display_name,
                pool: pool.clone(),
                clock: clock.clone(),
                auth,
                quota: ProviderQuota::new(Box::new(source), snapshots, clock.clone()),
                accounts: Accounts::new(pool, kit, clock),
                models_dev,
            }),
        }
    }

    /// A session's runtime, which starts its CLI processes through
    /// `placement` and keeps at most one of them between runs.
    pub fn process_runtime(&self, placement: Rc<dyn Placement>) -> Box<dyn ProviderRuntime> {
        Box::new(ClaudeCodeRuntime::new(self.shared.clone(), placement))
    }

    /// The entry's account operations with a sign-in that runs the CLI's own
    /// login on `machine` (`claude-code.md` § Accounts and sign-in).
    pub fn login_accounts(&self, machine: Arc<dyn AccountMachine>) -> Box<dyn SubscriptionAccounts> {
        let kit = ClaudeKit {
            machine: Some(machine),
            entry: self.shared.id.clone(),
            pool: self.shared.pool.clone(),
        };
        Box::new(Accounts::new(
            self.shared.pool.clone(),
            kit,
            self.shared.clock.clone(),
        ))
    }
}

impl Provider for ClaudeCodeProvider {
    fn capabilities(&self) -> Capabilities {
        Capabilities { process_host: true }
    }

    /// The account as its record holds it: signed in, unless its vendor
    /// signed it out; reading it starts nothing.
    fn auth_status(&self) -> BoxFuture<'_, AuthState> {
        Box::pin(async {
            match self.shared.auth.status().await {
                Ok(meta) => AuthState::Authenticated {
                    account_label: Some(meta.label),
                },
                Err(failure) => failure.state(),
            }
        })
    }

    /// Whether the CLI runs is known only when a request starts it on the
    /// Claude Code machine.
    fn runtime_state(&self) -> RuntimeState {
        RuntimeState::Unknown {
            message: Some("Claude Code runs on its machine when a request needs it".into()),
        }
    }

    fn list_models(&self) -> BoxFuture<'_, Result<ProviderModelList, CatalogError>> {
        Box::pin(models::list(&self.shared.models_dev))
    }

    /// A usage limit can succeed again at the reset its refused call names;
    /// any other failure is read by the standard HTTP rules.
    fn read_failure(
        &self,
        diagnostics: &ProviderErrorDiagnostics,
        received_at: Timestamp,
    ) -> ProviderFailureFacts {
        failure::read_failure(diagnostics, received_at)
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

/// Test support (feature `testing`): the session a new process resumes for
/// a request, for the tests of what sends this provider's requests.
#[cfg(feature = "testing")]
pub mod testing {
    use demi_provider_common::InferenceRequest;

    /// The entries of the session a new CLI process resumes for `request`,
    /// one JSON line each, after the `environment` entry, which describes
    /// the process's machine (`claude-code.md` § The session a process
    /// resumes), written at `timestamp`.
    pub fn session(request: &InferenceRequest, timestamp: &str) -> Vec<u8> {
        crate::session::SessionFile::new(
            &request.session_id,
            &request.items,
            &request.blocks,
            timestamp.to_owned(),
        )
        .history
    }
}
