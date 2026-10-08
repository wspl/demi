//! Logins (`providers.md` § Login and publication): each login runs the
//! family's login as a task of the backend's, reports what the user must do
//! while it waits, hands it the codes the user pastes, and publishes the
//! account when it completes. A login into a new entry authenticates against
//! a pool held in memory and publishes the entry with its account in one
//! transaction; a login into an existing entry holds the entry until it
//! ends. The vault alone ends a login the user does not finish: it stops the
//! family's login once the login's lifetime from its start has passed, and
//! that moment is the expiry the user is shown. Cancelling a login stops it
//! at once. A stopped login ends once it has released what it held, such as
//! the CLI process and directory of a Claude sign-in, and the cancel, the
//! expiry and shutdown wait for that. Its result is kept for ten minutes
//! after it ends.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use demi_provider_claude_code::AccountMachine;
use demi_provider_common::Secret;
use demi_provider_common::credentials::{LoginError, LoginIo, LoginKind, MemoryCredentialPool};
use demi_shared_types::{Clock, LoginPending, Timestamp};
use demi_web_api_protocol::ids::{CredentialId, LoginId, UserId};
use demi_web_api_protocol::providers::{CredentialKind, LoginState};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::entries::ProviderEntry;
use super::operations::{OperationGuard, ProviderOperations};
use crate::llm::assembly::{AssemblyError, LoginAccounts, ProviderAssembly};

/// How long a login waits for its user, and how long its result is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginTiming {
    /// From a device login's start to its end, whatever the vendor's code
    /// allows (`providers.md` § Login and publication).
    pub device_lifetime: Duration,
    /// From the start to the end of a login that takes a pasted code
    /// (`claude-code.md` § Accounts and sign-in).
    pub code_lifetime: Duration,
    pub retention: Duration,
}

impl Default for LoginTiming {
    fn default() -> Self {
        Self {
            device_lifetime: Duration::from_secs(10 * 60),
            code_lifetime: Duration::from_secs(15 * 60),
            retention: Duration::from_secs(10 * 60),
        }
    }
}

impl LoginTiming {
    fn lifetime(&self, kind: LoginKind) -> Duration {
        match kind {
            LoginKind::Device => self.device_lifetime,
            LoginKind::PastedCode => self.code_lifetime,
        }
    }
}

/// Why a login did not start.
#[derive(Debug, thiserror::Error)]
pub enum LoginRefusal {
    /// The family has no login.
    #[error("{0} has no login")]
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
pub struct LoginFlows {
    assembly: Arc<ProviderAssembly>,
    operations: Arc<ProviderOperations>,
    timing: LoginTiming,
    /// The wall clock the expiry a login shows is read from.
    clock: Arc<dyn Clock>,
    tasks: TaskTracker,
    closing: CancellationToken,
    /// A `std` mutex: the edge's requests and the login tasks update the
    /// flows from any thread, and no section awaits.
    flows: Mutex<HashMap<LoginId, Flow>>,
}

/// Why a pasted code was not handed to a login.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CodeRefusal {
    #[error("No such login")]
    NotFound,
    /// The login takes no pasted code, has shown no link yet, or ended.
    #[error("This sign-in is not waiting for a code")]
    NotWaiting,
}

struct Flow {
    owner: UserId,
    kind: LoginKind,
    state: LoginState,
    /// Where the codes the user pastes go, while the login runs.
    codes: mpsc::UnboundedSender<Secret>,
    cancel: CancellationToken,
    /// When the login ended; its result goes once the retention has passed.
    ended_at: Option<Instant>,
    /// Turns true when the login's task has recorded how it ended.
    ended: watch::Receiver<bool>,
}

/// How a login's task ends: at `deadline`, when `cancel` fires, or when
/// the flow finishes, whereupon it turns `ended` true.
struct Ending {
    deadline: Instant,
    cancel: CancellationToken,
    ended: watch::Sender<bool>,
    codes: mpsc::UnboundedReceiver<Secret>,
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
    pub fn new(
        assembly: Arc<ProviderAssembly>,
        operations: Arc<ProviderOperations>,
        timing: LoginTiming,
        clock: Arc<dyn Clock>,
    ) -> Arc<Self> {
        Arc::new(Self {
            assembly,
            operations,
            timing,
            clock,
            tasks: TaskTracker::new(),
            closing: CancellationToken::new(),
            flows: Mutex::default(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<LoginId, Flow>> {
        // A section only reads or replaces one flow.
        self.flows.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts `starter`'s login of `family`: into `existing`, or into a new
    /// entry of `owner`'s labelled `label`. A login that runs a process runs
    /// it on `machine`, the starter's. Only `starter` reads, answers and
    /// cancels it.
    pub async fn start(
        self: &Arc<Self>,
        owner: UserId,
        starter: UserId,
        family: String,
        label: String,
        existing: Option<ProviderEntry>,
        machine: Arc<dyn AccountMachine>,
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
        let (accounts, target) = match existing {
            Some(entry) => {
                let held = self
                    .operations
                    .reserve(&entry.id)
                    .ok_or(LoginRefusal::Busy)?;
                let accounts = self.assembly.login_accounts(&entry, machine).await?;
                let entry = Box::new(entry);
                (accounts, Target::Existing { entry, _held: held })
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
                let accounts = self.assembly.detached_login_accounts(
                    &family,
                    id.as_str(),
                    &label,
                    Arc::new(staged.clone()),
                    machine,
                )?;
                (
                    accounts,
                    Target::New {
                        family: family.clone(),
                        label,
                        staged,
                    },
                )
            }
        };
        let Some(kind) = accounts
            .accounts()
            .and_then(|accounts| accounts.capability().login)
        else {
            return Err(LoginRefusal::NoLoginFlow(family));
        };
        // The login's end, read on both clocks at its start: the task stops
        // the flow at `deadline`, and the user is shown `expires_at`.
        let lifetime = self.timing.lifetime(kind);
        let deadline = Instant::now() + lifetime;
        let expires_at = self
            .clock
            .now()
            .to_jiff()
            .saturating_add(lifetime)
            .map(Timestamp::truncate)
            .expect("adding a duration, unlike a calendar span, never fails");
        let cancel = self.closing.child_token();
        let (ended, ending) = watch::channel(false);
        let (codes, pasted) = mpsc::unbounded_channel();
        let flow = Flow {
            owner: starter,
            kind,
            state: LoginState::Pending {
                verification_url: None,
                user_code: None,
                needs_code: false,
                code_error: None,
                expires_at,
            },
            codes,
            cancel: cancel.clone(),
            ended_at: None,
            ended: ending,
        };
        self.lock().insert(id.clone(), flow);
        let ending = Ending {
            deadline,
            cancel,
            ended,
            codes: pasted,
        };
        let flows = self.clone();
        let login = id.clone();
        self.tasks
            .spawn(async move { flows.run(login, owner, accounts, target, ending).await });
        Ok(id)
    }

    /// Hands `code`, which the owner pasted, to the owner's login, which
    /// waits for one once it has shown its link; the refusal of an earlier
    /// code no longer stands. The code is never kept.
    pub fn submit_code(
        &self,
        id: &LoginId,
        owner: &UserId,
        code: Secret,
    ) -> Result<(), CodeRefusal> {
        let mut flows = self.lock();
        let flow = flows
            .get_mut(id)
            .filter(|flow| flow.owner == *owner)
            .ok_or(CodeRefusal::NotFound)?;
        let LoginState::Pending {
            needs_code: true,
            code_error,
            ..
        } = &mut flow.state
        else {
            return Err(CodeRefusal::NotWaiting);
        };
        if flow.cancel.is_cancelled() {
            return Err(CodeRefusal::NotWaiting);
        }
        flow.codes
            .send(code)
            .map_err(|_| CodeRefusal::NotWaiting)?;
        *code_error = None;
        Ok(())
    }

    /// The login's state, for its owner.
    pub fn state(&self, id: &LoginId, owner: &UserId) -> Option<LoginState> {
        self.prune();
        let flows = self.lock();
        flows
            .get(id)
            .filter(|flow| flow.owner == *owner)
            .map(|flow| flow.state.clone())
    }

    /// Cancels the owner's login and waits until it has ended; `false` for a
    /// login the owner does not have.
    pub async fn cancel(&self, id: &LoginId, owner: &UserId) -> bool {
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
    pub async fn close(&self) {
        self.closing.cancel();
        self.tasks.close();
        self.tasks.wait().await;
    }

    /// Drops the results that have been kept long enough.
    fn prune(&self) {
        let retention = self.timing.retention;
        self.lock().retain(|_, flow| {
            flow.ended_at
                .is_none_or(|ended| ended.elapsed() < retention)
        });
    }

    /// Runs the login until it completes, fails, expires or is cancelled, and
    /// records how it ended. A login that expires or is cancelled is stopped,
    /// and its end awaited, so that it has released what it held.
    async fn run(
        self: Arc<Self>,
        id: LoginId,
        owner: UserId,
        accounts: LoginAccounts,
        target: Target,
        ending: Ending,
    ) {
        let Ending {
            deadline,
            cancel,
            ended,
            codes,
        } = ending;
        let pending = {
            let flows = self.clone();
            let id = id.clone();
            move |pending: LoginPending| flows.pending(&id, pending)
        };
        let accounts = accounts
            .accounts()
            .expect("a login's provider has account operations");
        let stop = CancellationToken::new();
        let login = accounts.login(LoginIo {
            pending: &pending,
            codes,
            stop: stop.clone(),
        });
        tokio::pin!(login);
        let mut expired = false;
        let outcome = tokio::select! {
            outcome = &mut login => outcome,
            () = cancel.cancelled() => {
                stop.cancel();
                login.await
            }
            () = tokio::time::sleep_until(deadline) => {
                expired = true;
                stop.cancel();
                login.await
            }
        };
        let outcome = match outcome {
            Ok(account) => Ok(account),
            Err(LoginError::Stopped) if expired => Err("The login expired".to_owned()),
            Err(LoginError::Stopped) => Err("The login was cancelled".to_owned()),
            Err(error) => Err(error.to_string()),
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

    /// Stores what the vendor asked the user to do, while the login waits;
    /// the login still ends when it started to.
    fn pending(&self, id: &LoginId, pending: LoginPending) {
        let mut flows = self.lock();
        let Some(flow) = flows.get_mut(id) else {
            return;
        };
        let LoginState::Pending { expires_at, .. } = flow.state else {
            return;
        };
        if flow.cancel.is_cancelled() {
            return;
        }
        flow.state = LoginState::Pending {
            verification_url: Some(pending.verification_url),
            user_code: pending.user_code,
            needs_code: flow.kind == LoginKind::PastedCode,
            code_error: pending.code_error,
            expires_at,
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
            Target::New {
                family,
                label,
                staged,
            } => {
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use demi_backend_database::control::{ControlService, testing};
    use demi_backend_page_sync::SyncRegistry;
    use demi_provider_claude_code::{AccountWork, StartError};
    use demi_provider_common::Provider;
    use demi_provider_common::credentials::{
        AccountKit, AccountLabel, Accounts, AccountsCapability, NewAccount, SubscriptionAccounts,
    };
    use demi_provider_common::models_dev::ModelsDevClient;
    use demi_provider_common::testing::TokioClock;
    use demi_provider_common::{
        Capabilities, CatalogError, ProviderRuntime, RuntimeEnv, RuntimeError,
    };
    use demi_shared_types::{
        AuthState, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList, RuntimeState,
    };
    use demi_web_api_protocol::settings::InstanceMode;
    use futures_util::future::BoxFuture;

    use super::*;
    use crate::llm::catalog_cache::ModelCatalogCache;
    use crate::llm::families::{
        FamilyArgs, FamilyCredential, FamilyError, FamilyRegistry, ProviderFamily,
    };
    use crate::llm::vendors::VendorCatalog;
    use crate::vault::entries::Vault;
    use crate::vault::quotas::AccountQuotas;
    use crate::vault::seal::VaultKey;

    /// How long the vendor takes to name the code, after the login started.
    const VENDOR_ANSWERS_AFTER: Duration = Duration::from_secs(30);

    /// A device family whose user never confirms: its flow names a code a
    /// while after it starts, then waits, and counts the flows dropped.
    struct Unconfirmed {
        dropped: Arc<AtomicUsize>,
    }

    impl ProviderFamily for Unconfirmed {
        fn credential(&self) -> CredentialKind {
            CredentialKind::Subscription
        }

        fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
            let FamilyCredential::Subscription(subscription) = args.credential else {
                return Err(FamilyError::WrongCredential);
            };
            let kit = Kit {
                dropped: self.dropped.clone(),
            };
            Ok(Arc::new(Waiting(Accounts::new(
                subscription.pool,
                kit,
                args.clock,
            ))))
        }
    }

    struct Kit {
        dropped: Arc<AtomicUsize>,
    }

    /// Counts a flow dropped before it ended.
    struct Dropped(Arc<AtomicUsize>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl AccountKit for Kit {
        fn capability(&self) -> AccountsCapability {
            AccountsCapability {
                login: Some(LoginKind::Device),
            }
        }

        fn login<'a>(
            &'a self,
            io: LoginIo<'a>,
        ) -> Option<BoxFuture<'a, Result<NewAccount, LoginError>>> {
            let dropped = Dropped(self.dropped.clone());
            Some(Box::pin(async move {
                let waiting = async {
                    let _dropped = dropped;
                    tokio::time::sleep(VENDOR_ANSWERS_AFTER).await;
                    (io.pending)(LoginPending {
                        verification_url: "https://verify.example/device".into(),
                        user_code: Some("ABCD-1234".into()),
                        code_error: None,
                    });
                    std::future::pending::<()>().await;
                    Ok(NewAccount {
                        secret: "{}".into(),
                        label: AccountLabel {
                            label: "never".into(),
                            detail: None,
                            identity_key: None,
                        },
                    })
                };
                LoginIo::until_stopped(&io.stop, waiting).await
            }))
        }
    }

    /// The machine of a backend whose logins run no process.
    struct NoMachine;

    impl AccountMachine for NoMachine {
        fn run(&self, _: String, _: AccountWork) -> BoxFuture<'static, Result<(), StartError>> {
            unreachable!("a device login runs no process")
        }
    }

    /// A provider that only logs in.
    struct Waiting(Accounts<Kit>);

    impl Provider for Waiting {
        fn capabilities(&self) -> Capabilities {
            Capabilities::default()
        }

        fn auth_status(&self) -> BoxFuture<'_, AuthState> {
            unreachable!("a login reads no status")
        }

        fn runtime_state(&self) -> RuntimeState {
            unreachable!("a login reads no runtime state")
        }

        fn list_models(&self) -> BoxFuture<'_, Result<ProviderModelList, CatalogError>> {
            unreachable!("a login reads no models")
        }

        fn read_failure(
            &self,
            _diagnostics: &ProviderErrorDiagnostics,
            _received_at: Timestamp,
        ) -> ProviderFailureFacts {
            unreachable!("a login reads no failure")
        }

        fn accounts(&self) -> Option<&dyn SubscriptionAccounts> {
            Some(&self.0)
        }

        fn runtime(&self, _env: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
            unreachable!("a login builds no runtime")
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_login_ends_at_the_expiry_it_shows_from_its_start_and_its_result_goes_later() {
        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(
            &data.path().join("control.sqlite"),
            Arc::new(demi_shared_types::SystemClock),
        )
        .await
        .unwrap();
        let master = testing::master(&control).await.id;
        let clock = Arc::new(TokioClock::new("2026-09-24T08:00:00.000Z".parse().unwrap()));
        let vault = Vault::new(
            control.clone(),
            VaultKey::new([3; 32]),
            InstanceMode::Shared,
            SyncRegistry::default(),
        );
        let dropped = Arc::new(AtomicUsize::new(0));
        let families = FamilyRegistry::default().with(
            "device",
            Unconfirmed {
                dropped: dropped.clone(),
            },
        );
        let edge = tokio::runtime::Handle::current();
        let models_dev = ModelsDevClient::new(
            reqwest::Client::new(),
            ModelsDevClient::DEFAULT_URL.parse().unwrap(),
            clock.clone(),
        );
        let assembly = Arc::new(ProviderAssembly::new(
            vault.clone(),
            families,
            AccountQuotas::new(vault.clone(), edge.clone()),
            ModelCatalogCache::new(control.clone(), clock.clone(), edge),
            VendorCatalog::new(models_dev),
            reqwest::Client::new(),
            clock.clone(),
        ));
        let timing = LoginTiming::default();
        assert_eq!(timing.device_lifetime, Duration::from_secs(10 * 60));
        let flows = LoginFlows::new(
            assembly,
            Arc::new(ProviderOperations::default()),
            timing,
            clock.clone(),
        );

        let id = flows
            .start(
                master.clone(),
                master.clone(),
                "device".into(),
                "Device".into(),
                None,
                Arc::new(NoMachine),
            )
            .await
            .unwrap();
        // Nothing awaits between the start's last step and these reads.
        let started = Instant::now();
        let started_at = clock.now();
        let mut ended = flows.lock()[&id].ended.clone();

        // The vendor names its code later; the expiry shown still counts
        // from the start.
        tokio::time::sleep_until(started + timing.device_lifetime - Duration::from_millis(1)).await;
        let LoginState::Pending {
            verification_url,
            user_code,
            expires_at,
            ..
        } = flows.state(&id, &master).unwrap()
        else {
            panic!("the login ended before its expiry");
        };
        assert_eq!(
            (verification_url.as_deref(), user_code.as_deref()),
            (Some("https://verify.example/device"), Some("ABCD-1234"))
        );
        assert_eq!(
            expires_at.to_jiff().duration_since(started_at.to_jiff()),
            jiff::SignedDuration::from_mins(10)
        );
        assert_eq!(dropped.load(Ordering::SeqCst), 0);

        // The login ends at the moment it showed, and drops its flow.
        tokio::time::timeout(timing.device_lifetime, ended.wait_for(|ended| *ended))
            .await
            .expect("the login ends at its expiry")
            .unwrap();
        assert_eq!(Instant::now(), started + timing.device_lifetime);
        assert_eq!(clock.now(), expires_at);
        assert_eq!(
            flows.state(&id, &master),
            Some(LoginState::Failed {
                message: "The login expired".into()
            })
        );
        assert_eq!(dropped.load(Ordering::SeqCst), 1);

        // Its result stays for the retention, and then goes.
        let ended_at = Instant::now();
        tokio::time::sleep_until(ended_at + timing.retention - Duration::from_millis(1)).await;
        assert!(flows.state(&id, &master).is_some());
        tokio::time::sleep_until(ended_at + timing.retention).await;
        assert_eq!(flows.state(&id, &master), None);
        // An expired login stored nothing.
        assert!(vault.entries(master).await.unwrap().is_empty());
    }
}
