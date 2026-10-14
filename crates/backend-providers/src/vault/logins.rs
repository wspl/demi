//! Logins (`providers.md` § Login and publication): each login runs the
//! family's login as a task of the backend's, reports what it waits for and
//! what the user must do, hands it the codes the user pastes, and publishes
//! the account when it completes. A login into a new entry authenticates
//! against a pool held in memory and publishes the entry with its account
//! in one transaction; a login into an existing entry holds the entry until
//! it ends. The vault alone ends a login the user does not finish: it stops
//! the family's login once the login's lifetime from its start has passed,
//! and that moment is the expiry the user is shown. Cancelling a login
//! stops it at once. A stopped login ends once it has released what it
//! held, such as the CLI process and directory of a Claude sign-in, and the
//! cancel, the expiry and shutdown wait for that. Its result is kept for ten
//! minutes after it ends.
//!
//! A login is followed, never asked about: each change of its state, its
//! start, its stage, its link or code, a code handed over or refused, its
//! end and its drop, is announced on its starter's pages as the `logins`
//! part. The server log holds its start, each stage with the time the one
//! before took, and its end, under the request that started it, never a
//! link's or a code's contents.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use demi_backend_page_sync::{Part, SyncRegistry};
use demi_provider_claude_code::AccountMachine;
use demi_provider_common::Secret;
use demi_provider_common::credentials::{LoginError, LoginIo, LoginKind, MemoryCredentialPool};
use demi_shared_types::{Clock, LoginPending, LoginStage, RequestId, Timestamp};
use demi_web_api_protocol::ids::{CredentialId, LoginId, ProviderId, UserId};
use demi_web_api_protocol::providers::{CredentialKind, ListedLogin, LoginState};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Instrument as _;

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
    /// Another change holds the entry, or the backend is shutting down:
    /// the starter's own login into the entry, which it names, or a change
    /// of another kind, which names none.
    #[error("Another provider operation is still running")]
    Busy(Option<LoginId>),
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
    /// Where a login's change is announced to its starter's pages.
    sync: SyncRegistry,
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
    /// The user who started it, the only one who reads, answers and
    /// cancels it.
    owner: UserId,
    kind: LoginKind,
    /// The entry the login adds an account to; none for a login that makes
    /// a new entry.
    provider_id: Option<ProviderId>,
    family: String,
    /// The request that started it, which its failure names.
    request: RequestId,
    started: Instant,
    /// When the login reached its stage, for the time the stage took.
    stage_since: Instant,
    state: LoginState,
    /// Where the codes the user pastes go, while the login runs.
    codes: mpsc::UnboundedSender<Secret>,
    cancel: CancellationToken,
    /// Turns true when the login's task has recorded how it ended.
    ended: watch::Receiver<bool>,
}

impl Flow {
    fn listed(&self, id: &LoginId) -> ListedLogin {
        ListedLogin {
            id: id.clone(),
            provider_id: self.provider_id.clone(),
            provider_type: self.family.clone(),
            login: self.state.clone(),
        }
    }

    /// Moves the running login to `stage`, and logs the time the stage
    /// before took; `false` when it is there already or has ended.
    fn reach(&mut self, stage: LoginStage) -> bool {
        let LoginState::Pending { stage: current, .. } = &mut self.state else {
            return false;
        };
        if *current == stage {
            return false;
        }
        let now = Instant::now();
        tracing::info!(
            stage = stage.as_str(),
            previous = current.as_str(),
            took_ms = millis(now - self.stage_since),
            "the login reached a stage"
        );
        *current = stage;
        self.stage_since = now;
        true
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// How a login's family login ends: at `deadline`, when `cancel` fires, or
/// when the flow finishes; with the codes the user pastes meanwhile.
struct Ending {
    deadline: Instant,
    cancel: CancellationToken,
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
        sync: SyncRegistry,
    ) -> Arc<Self> {
        Arc::new(Self {
            assembly,
            operations,
            timing,
            clock,
            sync,
            tasks: TaskTracker::new(),
            closing: CancellationToken::new(),
            flows: Mutex::default(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<LoginId, Flow>> {
        // A section only reads or replaces one flow.
        self.flows.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Announces a change of `owner`'s logins to the owner's pages.
    fn announce(&self, owner: &UserId) {
        self.sync.mark(owner, Part::Logins);
    }

    /// Starts `starter`'s login of `family`, which the request `request`
    /// asked for: into `existing`, or into a new entry of `owner`'s
    /// labelled `label`. A login that runs a process runs it on `machine`,
    /// the starter's. Only `starter` reads, answers and cancels it.
    pub async fn start(
        self: &Arc<Self>,
        owner: UserId,
        starter: UserId,
        family: String,
        label: String,
        existing: Option<ProviderEntry>,
        machine: Arc<dyn AccountMachine>,
        request: RequestId,
    ) -> Result<LoginId, LoginRefusal> {
        if self.closing.is_cancelled() {
            return Err(LoginRefusal::Busy(None));
        }
        let registered = self.assembly.family(&family)?;
        if registered.credential() != CredentialKind::Subscription {
            return Err(LoginRefusal::NoLoginFlow(family));
        }
        let id = LoginId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        let provider_id = existing.as_ref().map(|entry| entry.id.clone());
        let (accounts, target) = match existing {
            Some(entry) => {
                let held = self
                    .operations
                    .reserve(&entry.id)
                    .ok_or_else(|| LoginRefusal::Busy(self.running_into(&starter, &entry.id)))?;
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
        let started = Instant::now();
        let deadline = started + lifetime;
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
        // Every event of the login is logged under the request that started
        // it, whose ID its failure names.
        let span = tracing::info_span!(
            "login",
            request_id = %request,
            user = %starter,
            login = %id,
            family = %family,
            provider = provider_id.as_ref().map(tracing::field::display),
        );
        let flow = Flow {
            owner: starter.clone(),
            kind,
            provider_id,
            family,
            request,
            started,
            stage_since: started,
            state: LoginState::Pending {
                stage: LoginStage::Preparing,
                verification_url: None,
                user_code: None,
                needs_code: false,
                codes_taken: 0,
                code_error: None,
                expires_at,
            },
            codes,
            cancel: cancel.clone(),
            ended: ending,
        };
        span.in_scope(|| tracing::info!("a login started"));
        self.lock().insert(id.clone(), flow);
        self.announce(&starter);
        let ending = Ending {
            deadline,
            cancel,
            codes: pasted,
        };
        let flows = self.clone();
        let login = id.clone();
        self.tasks.spawn(
            async move { flows.run(login, owner, accounts, target, ending, ended).await }
                .instrument(span),
        );
        Ok(id)
    }

    /// The running login of `starter`'s into the entry `entry`, which holds
    /// it.
    fn running_into(&self, starter: &UserId, entry: &ProviderId) -> Option<LoginId> {
        self.lock()
            .iter()
            .find(|(_, flow)| {
                flow.owner == *starter
                    && flow.provider_id.as_ref() == Some(entry)
                    && matches!(flow.state, LoginState::Pending { .. })
            })
            .map(|(id, _)| id.clone())
    }

    /// Hands `code`, which the owner pasted, to the owner's login, which
    /// waits for one once it has shown its link, as the next code it took;
    /// the refusal of an earlier code no longer stands. The code is never
    /// kept.
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
            codes_taken,
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
        *codes_taken += 1;
        *code_error = None;
        let taken = *codes_taken;
        drop(flows);
        tracing::info!(login = %id, code = taken, "a code was handed to the login");
        self.announce(owner);
        Ok(())
    }

    /// The login's state, for its owner.
    pub fn state(&self, id: &LoginId, owner: &UserId) -> Option<LoginState> {
        self.lock()
            .get(id)
            .filter(|flow| flow.owner == *owner)
            .map(|flow| flow.state.clone())
    }

    /// The logins `owner` started that run or ended within the retention,
    /// in the order they started.
    pub fn listed(&self, owner: &UserId) -> Vec<ListedLogin> {
        let flows = self.lock();
        let mut owned: Vec<_> = flows
            .iter()
            .filter(|(_, flow)| flow.owner == *owner)
            .collect();
        owned.sort_by_key(|(_, flow)| flow.started);
        owned.into_iter().map(|(id, flow)| flow.listed(id)).collect()
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

    /// Runs the login, records how it ended, whereupon it turns `ended`
    /// true, and drops its result once the retention has passed.
    async fn run(
        self: Arc<Self>,
        id: LoginId,
        owner: UserId,
        accounts: LoginAccounts,
        target: Target,
        ending: Ending,
        ended: watch::Sender<bool>,
    ) {
        let ended_as = self.attempt(&id, owner, accounts, target, ending).await;
        let starter = {
            let mut flows = self.lock();
            let flow = flows
                .get_mut(&id)
                .expect("a login's flow stays until its task drops it");
            let took_ms = millis(flow.started.elapsed());
            let LoginState::Pending { stage, .. } = &flow.state else {
                unreachable!("only the login's task ends it");
            };
            let stage = stage.as_str();
            flow.state = match ended_as {
                Ok((provider_id, credential_id)) => {
                    tracing::info!(took_ms, stage, "the login completed");
                    LoginState::Completed {
                        provider_id,
                        credential_id,
                    }
                }
                Err(message) => {
                    tracing::info!(took_ms, stage, reason = message, "the login failed");
                    LoginState::Failed {
                        message,
                        request_id: flow.request.clone(),
                    }
                }
            };
            flow.owner.clone()
        };
        self.announce(&starter);
        ended.send_replace(true);
        // Shutdown drops what is kept at once; nobody reads it any more.
        tokio::select! {
            () = tokio::time::sleep(self.timing.retention) => {}
            () = self.closing.cancelled() => {}
        }
        self.lock().remove(&id);
        self.announce(&starter);
    }

    /// Runs the family's login until it completes, fails, expires or is
    /// cancelled, and publishes its account; answers the entry and the
    /// account, or why it ended without one. A login that expires or is
    /// cancelled is stopped, and its end awaited, so that it has released
    /// what it held; the entry it held, the provider and the flow are
    /// released when this returns.
    async fn attempt(
        self: &Arc<Self>,
        id: &LoginId,
        owner: UserId,
        accounts: LoginAccounts,
        target: Target,
        ending: Ending,
    ) -> Result<(ProviderId, CredentialId), String> {
        let Ending {
            deadline,
            cancel,
            codes,
        } = ending;
        let pending = {
            let flows = self.clone();
            let id = id.clone();
            move |pending: LoginPending| flows.pending(&id, pending)
        };
        let stage = {
            let flows = self.clone();
            let id = id.clone();
            move |stage: LoginStage| flows.stage(&id, stage)
        };
        let accounts = accounts
            .accounts()
            .expect("a login's provider has account operations");
        let stop = CancellationToken::new();
        let login = accounts.login(LoginIo {
            pending: &pending,
            stage: &stage,
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
        match outcome {
            Ok(account) => self.publish(owner, target, account.id).await,
            Err(LoginError::Stopped) if expired => Err("The login expired".to_owned()),
            Err(LoginError::Stopped) => Err("The login was cancelled".to_owned()),
            Err(error) => Err(error.to_string()),
        }
    }

    /// Moves the login to `stage`, while it prepares.
    fn stage(&self, id: &LoginId, stage: LoginStage) {
        let mut flows = self.lock();
        let Some(flow) = flows.get_mut(id) else {
            return;
        };
        // Once the user is asked, the login waits for them until it ends.
        let asked = matches!(
            flow.state,
            LoginState::Pending {
                stage: LoginStage::WaitingForUser,
                ..
            }
        );
        if asked || flow.cancel.is_cancelled() || !flow.reach(stage) {
            return;
        }
        let owner = flow.owner.clone();
        drop(flows);
        self.announce(&owner);
    }

    /// Stores what the vendor asked the user to do, while the login waits;
    /// the login still ends when it started to. A refusal of a code before
    /// the last one the login took is no longer news: the user pasted
    /// another since.
    fn pending(&self, id: &LoginId, pending: LoginPending) {
        let mut flows = self.lock();
        let Some(flow) = flows.get_mut(id) else {
            return;
        };
        let LoginState::Pending {
            expires_at,
            codes_taken,
            ..
        } = flow.state
        else {
            return;
        };
        if flow.cancel.is_cancelled() {
            return;
        }
        let code_error = match pending.code_error {
            Some(refused) if refused.code == codes_taken => {
                tracing::info!(code = refused.code, "the login's sign-in refused a code");
                Some(refused.message)
            }
            Some(refused) => {
                tracing::info!(
                    code = refused.code,
                    codes_taken,
                    "the login's sign-in refused a code the user has pasted another after"
                );
                None
            }
            None => None,
        };
        let state = LoginState::Pending {
            stage: LoginStage::WaitingForUser,
            verification_url: Some(pending.verification_url),
            user_code: pending.user_code,
            needs_code: flow.kind == LoginKind::PastedCode,
            codes_taken,
            code_error,
            expires_at,
        };
        if state == flow.state {
            return;
        }
        flow.reach(LoginStage::WaitingForUser);
        flow.state = state;
        let owner = flow.owner.clone();
        drop(flows);
        self.announce(&owner);
    }

    /// Publishes the login's account: a new entry with its staged accounts,
    /// which a concurrent login of the same family may have won, or the
    /// existing entry, whose provider and catalog then start afresh.
    /// Answers the entry and the account, or why it failed.
    async fn publish(
        &self,
        owner: UserId,
        target: Target,
        account: String,
    ) -> Result<(ProviderId, CredentialId), String> {
        let account = CredentialId::try_from(account).map_err(|error| error.to_string())?;
        let provider_id = match target {
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
                    Ok(Some(entry)) => entry.id,
                    Ok(None) => return Err(LoginRefusal::Exists(family).to_string()),
                    Err(error) => return Err(error.to_string()),
                }
            }
            Target::Existing { entry, .. } => {
                self.assembly
                    .invalidate(&entry.id)
                    .await
                    .map_err(|error| error.to_string())?;
                entry.id
            }
        };
        Ok((provider_id, account))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use demi_backend_database::control::{ControlService, testing};
    use demi_backend_page_sync::Registration;
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
        AuthState, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList, RefusedCode,
        RuntimeState,
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

    /// A family whose login takes pasted codes and refuses the code the
    /// test names, when the test names it, then acknowledges it.
    struct Refusing {
        refusals: Arc<Mutex<Option<mpsc::UnboundedReceiver<(u32, tokio::sync::oneshot::Sender<()>)>>>>,
    }

    impl ProviderFamily for Refusing {
        fn credential(&self) -> CredentialKind {
            CredentialKind::Subscription
        }

        fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
            let FamilyCredential::Subscription(subscription) = args.credential else {
                return Err(FamilyError::WrongCredential);
            };
            let kit = RefusingKit {
                refusals: self.refusals.clone(),
            };
            Ok(Arc::new(Waiting(Accounts::new(
                subscription.pool,
                kit,
                args.clock,
            ))))
        }
    }

    struct RefusingKit {
        refusals: Arc<Mutex<Option<mpsc::UnboundedReceiver<(u32, tokio::sync::oneshot::Sender<()>)>>>>,
    }

    impl AccountKit for RefusingKit {
        fn capability(&self) -> AccountsCapability {
            AccountsCapability {
                login: Some(LoginKind::PastedCode),
            }
        }

        fn login<'a>(
            &'a self,
            io: LoginIo<'a>,
        ) -> Option<BoxFuture<'a, Result<NewAccount, LoginError>>> {
            let mut refusals = self.refusals.lock().unwrap().take().expect("one login");
            Some(Box::pin(async move {
                let link = || "https://sign-in.example/authorize".to_owned();
                (io.pending)(LoginPending {
                    verification_url: link(),
                    user_code: None,
                    code_error: None,
                });
                let refusing = async {
                    // The codes stay taken while the login runs; the test
                    // names the refusals itself.
                    let _codes = &io.codes;
                    while let Some((code, refused)) = refusals.recv().await {
                        (io.pending)(LoginPending {
                            verification_url: link(),
                            user_code: None,
                            code_error: Some(RefusedCode {
                                code,
                                message: format!("Invalid code {code}"),
                            }),
                        });
                        // The test waits for it.
                        let _ = refused.send(());
                    }
                    std::future::pending().await
                };
                LoginIo::until_stopped(&io.stop, refusing).await
            }))
        }
    }

    // Instant: nothing waits for time.
    #[tokio::test]
    async fn a_refused_code_shows_only_while_no_later_code_was_taken() {
        let (refuse, refusals) = mpsc::unbounded_channel();
        let families = FamilyRegistry::default().with(
            "pasted",
            Refusing {
                refusals: Arc::new(Mutex::new(Some(refusals))),
            },
        );
        let clock = Arc::new(TokioClock::new("2026-09-24T08:00:00.000Z".parse().unwrap()));
        let Backend {
            _data,
            sync,
            flows,
            master,
            ..
        } = backend(families, clock, LoginTiming::default()).await;
        let page = sync.watch(&master);
        let id = flows
            .start(
                master.clone(),
                master.clone(),
                "pasted".into(),
                "Pasted".into(),
                None,
                Arc::new(NoMachine),
                RequestId::try_from("req_starting".to_owned()).unwrap(),
            )
            .await
            .unwrap();
        while !matches!(
            &announced(&page, &flows, &master).await[..],
            [ListedLogin { login: LoginState::Pending { verification_url: Some(_), .. }, .. }]
        ) {}
        let refused = |code| {
            let (acknowledged, acknowledgement) = tokio::sync::oneshot::channel();
            refuse.send((code, acknowledged)).unwrap();
            async { acknowledgement.await.unwrap() }
        };
        let shown = || match flows.state(&id, &master) {
            Some(LoginState::Pending {
                codes_taken,
                code_error,
                ..
            }) => (codes_taken, code_error),
            other => panic!("the login ended: {other:?}"),
        };

        // The user pastes a second code before the sign-in judges the
        // first: the first's refusal is not shown over the second.
        for code in ["first", "second"] {
            flows
                .submit_code(&id, &master, Secret::try_from(code.to_owned()).unwrap())
                .unwrap();
        }
        assert_eq!(shown(), (2, None));
        refused(1).await;
        assert_eq!(shown(), (2, None));

        // The second's refusal is.
        refused(2).await;
        assert_eq!(shown(), (2, Some("Invalid code 2".to_owned())));
        flows.close().await;
    }

    /// The machine of a backend whose logins run no process.
    struct NoMachine;

    impl AccountMachine for NoMachine {
        fn run(&self, _: String, _: AccountWork) -> BoxFuture<'static, Result<(), StartError>> {
            unreachable!("a device login runs no process")
        }
    }

    /// A provider that only logs in.
    struct Waiting<K>(Accounts<K>);

    impl<K: AccountKit + 'static> Provider for Waiting<K> {
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

    /// The vault's logins over a new control database with its master, for
    /// `families`.
    struct Backend {
        _data: tempfile::TempDir,
        vault: Vault,
        sync: SyncRegistry,
        flows: Arc<LoginFlows>,
        master: UserId,
    }

    async fn backend(families: FamilyRegistry, clock: Arc<TokioClock>, timing: LoginTiming) -> Backend {
        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(
            &data.path().join("control.sqlite"),
            Arc::new(demi_shared_types::SystemClock),
        )
        .await
        .unwrap();
        let master = testing::master(&control).await.id;
        let sync = SyncRegistry::default();
        let vault = Vault::new(
            control.clone(),
            VaultKey::new([3; 32]),
            InstanceMode::Shared,
            sync.clone(),
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
        let flows = LoginFlows::new(
            assembly,
            Arc::new(ProviderOperations::default()),
            timing,
            clock,
            sync.clone(),
        );
        Backend {
            _data: data,
            vault,
            sync,
            flows,
            master,
        }
    }

    /// The owner's logins as the vault announces them next on a page's
    /// channel.
    async fn announced(
        channel: &Registration,
        flows: &LoginFlows,
        owner: &UserId,
    ) -> Vec<ListedLogin> {
        // A day of the paused clock passes at once: a change the vault does
        // not announce fails here rather than hanging.
        let announced = async {
            loop {
                channel.marked().await;
                if channel.take().parts.contains(&Part::Logins) {
                    return flows.listed(owner);
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(24 * 60 * 60), announced)
            .await
            .expect("the vault announces the login's next change")
    }

    // Instant: the paused clock jumps to each of the login's moments.
    #[tokio::test(start_paused = true)]
    async fn a_login_announces_each_change_ends_at_the_expiry_it_shows_and_its_result_goes_later()
    {
        let clock = Arc::new(TokioClock::new("2026-09-24T08:00:00.000Z".parse().unwrap()));
        let dropped = Arc::new(AtomicUsize::new(0));
        let families = FamilyRegistry::default().with(
            "device",
            Unconfirmed {
                dropped: dropped.clone(),
            },
        );
        let timing = LoginTiming::default();
        assert_eq!(timing.device_lifetime, Duration::from_secs(10 * 60));
        let Backend {
            _data,
            vault,
            sync,
            flows,
            master,
        } = backend(families, clock.clone(), timing).await;
        let page = sync.watch(&master);
        let request = RequestId::try_from("req_starting".to_owned()).unwrap();

        // The start is announced, preparing.
        let id = flows
            .start(
                master.clone(),
                master.clone(),
                "device".into(),
                "Device".into(),
                None,
                Arc::new(NoMachine),
                request.clone(),
            )
            .await
            .unwrap();
        // Nothing awaits between the start's last step and these reads.
        let started = Instant::now();
        let started_at = clock.now();
        let [started_login] = &announced(&page, &flows, &master).await[..] else {
            panic!("the start announced other logins");
        };
        assert_eq!(
            (
                &started_login.id,
                started_login.provider_id.as_ref(),
                started_login.provider_type.as_str()
            ),
            (&id, None, "device")
        );
        let LoginState::Pending {
            stage: LoginStage::Preparing,
            verification_url: None,
            expires_at,
            ..
        } = started_login.login
        else {
            panic!("the login did not start preparing: {started_login:?}");
        };
        assert_eq!(
            expires_at.to_jiff().duration_since(started_at.to_jiff()),
            jiff::SignedDuration::from_mins(10)
        );

        // The vendor names its code later, and the page learns it then; the
        // expiry shown still counts from the start.
        let [shown] = &announced(&page, &flows, &master).await[..] else {
            panic!("the code announced other logins");
        };
        assert_eq!(Instant::now(), started + VENDOR_ANSWERS_AFTER);
        assert_eq!(
            shown.login,
            LoginState::Pending {
                stage: LoginStage::WaitingForUser,
                verification_url: Some("https://verify.example/device".into()),
                user_code: Some("ABCD-1234".into()),
                needs_code: false,
                codes_taken: 0,
                code_error: None,
                expires_at,
            }
        );
        assert_eq!(flows.state(&id, &master), Some(shown.login.clone()));

        // The login ends at the moment it showed, drops its flow, and the
        // page learns its failure with the request that started it.
        let [ended] = &announced(&page, &flows, &master).await[..] else {
            panic!("the end announced other logins");
        };
        assert_eq!(Instant::now(), started + timing.device_lifetime);
        assert_eq!(clock.now(), expires_at);
        assert_eq!(
            ended.login,
            LoginState::Failed {
                message: "The login expired".into(),
                request_id: request,
            }
        );
        assert_eq!(dropped.load(Ordering::SeqCst), 1);

        // Its result stays for the retention, and its drop is announced.
        let ended_at = Instant::now();
        assert_eq!(announced(&page, &flows, &master).await, []);
        assert_eq!(Instant::now(), ended_at + timing.retention);
        assert_eq!(flows.state(&id, &master), None);
        // An expired login stored nothing.
        assert!(vault.entries(master).await.unwrap().is_empty());
        flows.close().await;
    }
}
