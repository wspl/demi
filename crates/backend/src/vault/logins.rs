//! Device logins (`providers.md` § Login and publication): each login runs
//! the family's device flow as a task of the backend's, reports what the
//! user must do while it waits, and publishes the account when it
//! completes. A login into a new entry authenticates against a pool held in
//! memory and publishes the entry with its account in one transaction; a
//! login into an existing entry holds the entry until it ends. A login
//! expires after ten minutes, cancelling it stops it at once, and its result
//! is kept for ten minutes after it ends.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use demi_core::LoginPending;
use demi_provider::Provider;
use demi_provider::credentials::MemoryCredentialPool;
use demi_web_api::ids::{CredentialId, LoginId, UserId};
use demi_web_api::providers::{CredentialKind, LoginState};
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::entries::ProviderEntry;
use super::operations::{OperationGuard, ProviderOperations};
use crate::llm::assembly::{AssemblyError, ProviderAssembly};

/// How long a login waits for its user, and how long its result is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginTiming {
    pub lifetime: Duration,
    pub retention: Duration,
}

impl Default for LoginTiming {
    fn default() -> Self {
        Self {
            lifetime: Duration::from_secs(10 * 60),
            retention: Duration::from_secs(10 * 60),
        }
    }
}

/// Why a login did not start.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LoginRefusal {
    /// The family has no device login.
    #[error("{0} has no device login")]
    NoLoginFlow(String),
    /// The owner already holds the family's subscription entry.
    #[error("This scope already has a {0} subscription")]
    Exists(String),
    /// Another change holds the entry, or the backend is shutting down.
    #[error("Another provider operation is still running")]
    Busy,
    #[error(transparent)]
    Assembly(#[from] AssemblyError),
}

/// The logins of the backend.
pub(crate) struct LoginFlows {
    assembly: Arc<ProviderAssembly>,
    operations: Arc<ProviderOperations>,
    timing: LoginTiming,
    tasks: TaskTracker,
    closing: CancellationToken,
    /// A `std` mutex: the edge's requests and the login tasks update the
    /// flows from any thread, and no section awaits.
    flows: Mutex<HashMap<LoginId, Flow>>,
}

struct Flow {
    owner: UserId,
    state: LoginState,
    cancel: CancellationToken,
    /// When the login ended; its result goes once the retention has passed.
    ended_at: Option<Instant>,
    /// Turns true when the login's task has recorded how it ended.
    ended: watch::Receiver<bool>,
}

/// Where a login publishes its account.
enum Target {
    /// A new entry, published with the staged pool's accounts.
    New {
        family: String,
        label: String,
        staged: MemoryCredentialPool,
    },
    /// An existing entry, held until the login ends.
    Existing {
        entry: Box<ProviderEntry>,
        _held: OperationGuard,
    },
}

impl LoginFlows {
    pub(crate) fn new(
        assembly: Arc<ProviderAssembly>,
        operations: Arc<ProviderOperations>,
        timing: LoginTiming,
    ) -> Arc<Self> {
        Arc::new(Self {
            assembly,
            operations,
            timing,
            tasks: TaskTracker::new(),
            closing: CancellationToken::new(),
            flows: Mutex::default(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<LoginId, Flow>> {
        // A section only reads or replaces one flow.
        self.flows.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts `starter`'s device login of `family`: into `existing`, or into
    /// a new entry of `owner`'s labelled `label`. Only `starter` reads and
    /// cancels it.
    pub(crate) async fn start(
        self: &Arc<Self>,
        owner: UserId,
        starter: UserId,
        family: String,
        label: String,
        existing: Option<ProviderEntry>,
    ) -> Result<LoginId, LoginRefusal> {
        if self.closing.is_cancelled() {
            return Err(LoginRefusal::Busy);
        }
        let registered = self.assembly.family(&family)?;
        if registered.credential() != CredentialKind::Subscription {
            return Err(LoginRefusal::NoLoginFlow(family));
        }
        self.prune();
        let id = LoginId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        let (provider, target) = match existing {
            Some(entry) => {
                let held = self.operations.reserve(&entry.id).ok_or(LoginRefusal::Busy)?;
                let provider = self.assembly.provider_for(&entry).await?;
                let entry = Box::new(entry);
                (provider, Target::Existing { entry, _held: held })
            }
            None => {
                let entries = self
                    .assembly
                    .vault()
                    .entries(owner.clone())
                    .await
                    .map_err(AssemblyError::from)?;
                if entries.iter().any(|entry| entry.family == family) {
                    return Err(LoginRefusal::Exists(family));
                }
                let staged = MemoryCredentialPool::new();
                let provider = self
                    .assembly
                    .detached(&family, id.as_str(), &label, Arc::new(staged.clone()))?;
                (
                    provider,
                    Target::New {
                        family: family.clone(),
                        label,
                        staged,
                    },
                )
            }
        };
        if !provider.accounts().is_some_and(|accounts| accounts.capability().login) {
            return Err(LoginRefusal::NoLoginFlow(family));
        }
        let cancel = self.closing.child_token();
        let (ended, ending) = watch::channel(false);
        let flow = Flow {
            owner: starter,
            state: LoginState::Pending {
                verification_url: None,
                user_code: None,
                expires_at: None,
            },
            cancel: cancel.clone(),
            ended_at: None,
            ended: ending,
        };
        self.lock().insert(id.clone(), flow);
        let flows = self.clone();
        let login = id.clone();
        self.tasks
            .spawn(async move { flows.run(login, owner, provider, target, cancel, ended).await });
        Ok(id)
    }

    /// The login's state, for its owner.
    pub(crate) fn state(&self, id: &LoginId, owner: &UserId) -> Option<LoginState> {
        self.prune();
        let flows = self.lock();
        flows
            .get(id)
            .filter(|flow| flow.owner == *owner)
            .map(|flow| flow.state.clone())
    }

    /// Cancels the owner's login and waits until it has ended; `false` for a
    /// login the owner does not have.
    pub(crate) async fn cancel(&self, id: &LoginId, owner: &UserId) -> bool {
        let (cancel, mut ended) = {
            let flows = self.lock();
            match flows.get(id).filter(|flow| flow.owner == *owner) {
                Some(flow) => (flow.cancel.clone(), flow.ended.clone()),
                None => return false,
            }
        };
        cancel.cancel();
        // A task that is gone has recorded its end already.
        let _ = ended.wait_for(|ended| *ended).await;
        true
    }

    /// Cancels every login and waits for them, at shutdown.
    pub(crate) async fn close(&self) {
        self.closing.cancel();
        self.tasks.close();
        self.tasks.wait().await;
    }

    /// Drops the results that have been kept long enough.
    fn prune(&self) {
        let retention = self.timing.retention;
        self.lock()
            .retain(|_, flow| flow.ended_at.is_none_or(|ended| ended.elapsed() < retention));
    }

    /// Runs the login until it completes, fails, expires or is cancelled, and
    /// records how it ended.
    async fn run(
        self: Arc<Self>,
        id: LoginId,
        owner: UserId,
        provider: Arc<dyn Provider>,
        target: Target,
        cancel: CancellationToken,
        ended: watch::Sender<bool>,
    ) {
        let pending = {
            let flows = self.clone();
            let id = id.clone();
            move |pending: LoginPending| flows.pending(&id, pending)
        };
        let accounts = provider.accounts().expect("a login's provider has account operations");
        let outcome = tokio::select! {
            () = cancel.cancelled() => Err("The login was cancelled".to_owned()),
            outcome = tokio::time::timeout(self.timing.lifetime, accounts.login(&pending)) => match outcome {
                Err(_) => Err("The login expired".to_owned()),
                Ok(Err(error)) => Err(error.to_string()),
                Ok(Ok(account)) => Ok(account),
            },
        };
        let state = match outcome {
            Ok(account) => self.publish(owner, target, account.id).await,
            Err(message) => LoginState::Failed { message },
        };
        if let Some(flow) = self.lock().get_mut(&id) {
            flow.state = state;
            flow.ended_at = Some(Instant::now());
        }
        ended.send_replace(true);
    }

    /// Stores what the vendor asked the user to do, while the login waits.
    fn pending(&self, id: &LoginId, pending: LoginPending) {
        let mut flows = self.lock();
        let Some(flow) = flows.get_mut(id) else {
            return;
        };
        if flow.cancel.is_cancelled() || !matches!(flow.state, LoginState::Pending { .. }) {
            return;
        }
        flow.state = LoginState::Pending {
            verification_url: Some(pending.verification_url),
            user_code: pending.user_code,
            expires_at: pending.expires_at,
        };
    }

    /// Publishes the login's account: a new entry with its staged accounts,
    /// which a concurrent login of the same family may have won, or the
    /// existing entry, whose provider and catalog then start afresh.
    async fn publish(&self, owner: UserId, target: Target, account: String) -> LoginState {
        let account = match CredentialId::try_from(account) {
            Ok(account) => account,
            Err(error) => {
                return LoginState::Failed {
                    message: error.to_string(),
                };
            }
        };
        let published = match target {
            Target::New { family, label, staged } => {
                let created = self
                    .assembly
                    .vault()
                    .create_subscription(owner, family.clone(), label, &staged)
                    .await;
                match created {
                    Ok(Some(entry)) => Ok(entry.id),
                    Ok(None) => Err(LoginRefusal::Exists(family).to_string()),
                    Err(error) => Err(error.to_string()),
                }
            }
            Target::Existing { entry, .. } => match self.assembly.invalidate(&entry.id).await {
                Ok(()) => Ok(entry.id),
                Err(error) => Err(error.to_string()),
            },
        };
        match published {
            Ok(provider_id) => LoginState::Completed {
                provider_id,
                credential_id: account,
            },
            Err(message) => LoginState::Failed { message },
        }
    }
}
