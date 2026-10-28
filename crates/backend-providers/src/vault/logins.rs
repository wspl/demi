//! Logins: each login does four things:
//!
//! - Runs the family's login as a task of the backend's.
//! - Reports what it waits for and what the user must do.
//! - Hands the family's login the codes the user pastes.
//! - Publishes the account when it completes.
//!
//! A login into a new entry authenticates against a pool held in memory and
//! publishes the entry with its account in one transaction. A login into an
//! existing entry holds the entry until it ends.
//!
//! The vault alone ends a login the user does not finish. It stops the family's
//! login once the login's lifetime from its start has passed, and that moment
//! is the expiry the user is shown.
//!
//! Cancelling a login stops it at once. A stopped login ends once it has
//! released what it held, such as the CLI process and directory of a Claude
//! sign-in, and the cancel, the expiry and shutdown wait for that. A login's
//! result is kept for ten minutes after it ends.
//!
//! A login is followed, never asked about. Each change of its state is
//! announced on its starter's pages as the `logins` part: its start, its stage,
//! its link or code, a code handed over or refused, its end and its drop.
//!
//! The server log holds its start, each stage with the time the one before
//! took, and its end, under the request that started it, never a link's or a
//! code's contents.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use demi_backend_page_sync::{Part, SyncRegistry};
use demi_provider_common::Secret;
use demi_provider_common::credentials::{LoginError, LoginIo, LoginKind, MemoryCredentialPool};
use demi_shared_types::{Clock, DownloadProgress, LoginPending, LoginStage, RequestId, Timestamp};
use demi_web_api_protocol::ids::{CredentialId, LoginId, ProviderId, UserId};
use demi_web_api_protocol::providers::{CredentialKind, ListedLogin, LoginState};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Instrument as _;

use demi_provider_registry::entry::ProviderEntry;
use demi_provider_registry::registry::{LoginAccounts, ProviderRegistry, RegistryError};
use demi_provider_registry::store::StoreError;

use super::entries::Vault;
use super::operations::{Held, HeldBy, OperationGuard, ProviderOperations};

/// How long a login waits for its user, and how long its result is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginTiming {
    /// From a device login's start to its end, whatever the vendor's code
    /// allows.
    pub device_lifetime: Duration,
    /// From the start to the end of a login that takes a pasted code
    /// (`crates/provider-claude-code/src/account.rs`).
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
    /// The starter infers with the master's subscriptions, which a shared
    /// instance gives every user when `DEMI_SHARED_SUBSCRIPTIONS` says so.
    #[error("Subscriptions are shared by the instance owner")]
    NotOffered,
    /// The owner already holds the family's subscription entry.
    #[error("This scope already has a {0} subscription")]
    Exists(String),
    /// Another change holds the entry, or the backend is shutting down:
    /// the starter's own login into the entry, which it names, or a change
    /// of another kind, which names none.
    #[error("Another provider operation is still running")]
    Busy(Option<LoginId>),
    #[error(transparent)]
    Registry(#[from] RegistryError),
}

/// The logins of the backend.
pub struct LoginFlows {
    vault: Vault,
    provider_registry: Arc<ProviderRegistry>,
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
    /// How the user finishes it, once the family's login is made; none
    /// while the login prepares it.
    kind: Option<LoginKind>,
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
        let LoginState::Pending {
            stage: current,
            download,
            ..
        } = &mut self.state
        else {
            return false;
        };
        if *current == stage {
            return false;
        }
        // A download belongs to the stage that waits for it.
        *download = None;
        let now = Instant::now();
        tracing::info!(
            event = "login.stage_reached",
            stage = stage.as_str(),
            previous = current.as_str(),
            ms = millis(now - self.stage_since),
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
    /// The entry the login adds to or makes, held until it ends.
    held: OperationGuard,
}

/// How a login ended without an account.
enum Unfinished {
    Cancelled,
    Failed(String),
}

/// Where a login publishes its account.
enum Target {
    /// A new entry, published with the staged pool's accounts.
    New {
        family: String,
        label: String,
        staged: MemoryCredentialPool,
    },
    /// An existing entry.
    Existing { entry: Box<ProviderEntry> },
}

impl LoginFlows {
    pub fn new(
        vault: Vault,
        provider_registry: Arc<ProviderRegistry>,
        operations: Arc<ProviderOperations>,
        timing: LoginTiming,
        clock: Arc<dyn Clock>,
        sync: SyncRegistry,
    ) -> Arc<Self> {
        Arc::new(Self {
            vault,
            provider_registry,
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
    /// asked for: into `existing`, or into a new entry of the starter's
    /// labelled `label`. A login that runs a process runs it on the Claude
    /// Code machine. Only `starter` reads, answers and cancels it.
    pub async fn start(
        self: &Arc<Self>,
        starter: UserId,
        family: String,
        label: String,
        existing: Option<ProviderEntry>,
        request: RequestId,
    ) -> Result<LoginId, LoginRefusal> {
        if self.closing.is_cancelled() {
            return Err(LoginRefusal::Busy(None));
        }
        let registered = self.provider_registry.family(&family)?;
        if registered.credential() != CredentialKind::Subscription {
            return Err(LoginRefusal::NoLoginFlow(family));
        }
        let owners = self
            .vault
            .owners(&starter)
            .await
            .map_err(|error| RegistryError::from(StoreError::new(error)))?;
        if !owners.manages(CredentialKind::Subscription) {
            return Err(LoginRefusal::NotOffered);
        }
        let owner = starter.clone();
        let id = LoginId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        let provider_id = existing.as_ref().map(|entry| entry.id.clone());
        // The entry the login adds to or makes is held from here, so a
        // second start meanwhile is refused and names this login to its
        // starter.
        let held = match &existing {
            Some(entry) => Held::Entry(entry.id.clone()),
            None => Held::NewEntry {
                owner: owner.clone(),
                family: family.clone(),
            },
        };
        let by = HeldBy {
            login: id.clone(),
            starter: starter.clone(),
        };
        let held = self.operations.hold(held, Some(by)).map_err(|holder| {
            LoginRefusal::Busy(
                holder
                    .filter(|holder| holder.starter == starter)
                    .map(|holder| holder.login),
            )
        })?;
        // The login is listed and announced from here, so a page that a
        // second start sent to it finds it, and learns its end, a failure
        // of its start included.
        let started = Instant::now();
        let started_at = self.clock.now();
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
            kind: None,
            provider_id,
            family: family.clone(),
            request,
            started,
            stage_since: started,
            state: LoginState::Pending {
                stage: LoginStage::Preparing,
                download: None,
                verification_url: None,
                user_code: None,
                needs_code: false,
                codes_taken: 0,
                code_error: None,
                // Until the login knows its kind, and so its lifetime.
                expires_at: None,
            },
            codes,
            cancel: cancel.clone(),
            ended: ending,
        };
        span.in_scope(|| tracing::info!(event = "login.started", "a login started"));
        self.lock().insert(id.clone(), flow);
        self.announce(&starter);
        let prepared = async {
            let (accounts, target) = match existing {
                Some(entry) => {
                    let accounts = self.provider_registry.login_accounts(&entry).await?;
                    let entry = Box::new(entry);
                    (accounts, Target::Existing { entry })
                }
                None => {
                    let entries = self
                        .vault
                        .entries(owner.clone())
                        .await
                        .map_err(|error| RegistryError::from(StoreError::new(error)))?;
                    if entries.iter().any(|entry| entry.family == family) {
                        return Err(LoginRefusal::Exists(family.clone()));
                    }
                    let staged = MemoryCredentialPool::new();
                    let accounts = self.provider_registry.detached_login_accounts(
                        &family,
                        id.as_str(),
                        &label,
                        Arc::new(staged.clone()),
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
            let kind = accounts
                .accounts()
                .and_then(|accounts| accounts.capability().login)
                .ok_or_else(|| LoginRefusal::NoLoginFlow(family.clone()))?;
            Ok((accounts, target, kind))
        }
        .instrument(span.clone())
        .await;
        let (accounts, target, kind) = match prepared {
            Ok(prepared) => prepared,
            Err(refusal) => {
                // The start answers the refusal; the login ends with it,
                // as every page that shows it learns.
                span.in_scope(|| self.end(&id, Err(Unfinished::Failed(refusal.to_string()))));
                ended.send_replace(true);
                self.tasks
                    .spawn(self.clone().retire(id, starter).instrument(span));
                return Err(refusal);
            }
        };
        // The login's end, read on both clocks at its start: the task stops
        // the flow at `deadline`, and the user is shown `expires_at`, which
        // is announced.
        let lifetime = self.timing.lifetime(kind);
        let deadline = started + lifetime;
        let expires_at = started_at
            .to_jiff()
            .saturating_add(lifetime)
            .map(Timestamp::truncate)
            .expect("adding a duration, unlike a calendar span, never fails");
        if let Some(flow) = self.lock().get_mut(&id) {
            flow.kind = Some(kind);
            if let LoginState::Pending { expires_at: shown, .. } = &mut flow.state {
                *shown = Some(expires_at);
            }
        }
        self.announce(&starter);
        let ending = Ending {
            deadline,
            cancel,
            codes: pasted,
            held,
        };
        let flows = self.clone();
        let login = id.clone();
        self.tasks.spawn(
            async move { flows.run(login, owner, accounts, target, ending, ended).await }
                .instrument(span),
        );
        Ok(id)
    }

    /// Hands `code`, which the owner pasted, to the owner's login, which waits
    /// for one once it has shown its link.
    ///
    /// Answers its number among the codes the login took, counting from one,
    /// which a refusal of it names. The refusal of an earlier code no longer
    /// stands. The code is never kept.
    pub fn submit_code(
        &self,
        id: &LoginId,
        owner: &UserId,
        code: Secret,
    ) -> Result<u32, CodeRefusal> {
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
        tracing::info!(event = "login.code_handed", login = %id, attempt = taken, "a code was handed to the login");
        self.announce(owner);
        Ok(taken)
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
        let starter = self.end(&id, ended_as);
        ended.send_replace(true);
        self.retire(id, starter).await;
    }

    /// Records how the login ended, announces it, and answers its starter.
    fn end(&self, id: &LoginId, ended_as: Result<(ProviderId, CredentialId), Unfinished>) -> UserId {
        let starter = {
            let mut flows = self.lock();
            let flow = flows
                .get_mut(id)
                .expect("a login's flow stays until it is retired");
            let ms = millis(flow.started.elapsed());
            let LoginState::Pending { stage, .. } = &flow.state else {
                unreachable!("a login ends once");
            };
            let stage = stage.as_str();
            let (login, user) = (id, &flow.owner);
            flow.state = match ended_as {
                Ok((provider_id, credential_id)) => {
                    tracing::info!(event = "login.ended", %login, %user, outcome = "completed", ms, stage, "the login completed");
                    LoginState::Completed {
                        provider_id,
                        credential_id,
                    }
                }
                Err(Unfinished::Cancelled) => {
                    tracing::info!(event = "login.ended", %login, %user, outcome = "cancelled", ms, stage, "the login was cancelled");
                    LoginState::Cancelled
                }
                Err(Unfinished::Failed(message)) => {
                    tracing::info!(event = "login.ended", %login, %user, outcome = "failed", ms, stage, "the login failed");
                    LoginState::Failed {
                        message,
                        request_id: flow.request.clone(),
                    }
                }
            };
            flow.owner.clone()
        };
        self.announce(&starter);
        starter
    }

    /// Drops the ended login's result once the retention has passed, and
    /// announces it.
    async fn retire(self: Arc<Self>, id: LoginId, starter: UserId) {
        // Shutdown drops what is kept at once; nobody reads it any more.
        tokio::select! {
            () = tokio::time::sleep(self.timing.retention) => {}
            () = self.closing.cancelled() => {}
        }
        self.lock().remove(&id);
        self.announce(&starter);
    }

    /// Runs the family's login until it completes, fails, expires or is
    /// cancelled, and publishes its account. Answers the entry and the account,
    /// or why it ended without one.
    ///
    /// A login that expires or is cancelled is stopped, and its end awaited, so
    /// that it has released what it held. The entry it held, the provider and
    /// the flow are released when this returns.
    async fn attempt(
        self: &Arc<Self>,
        id: &LoginId,
        owner: UserId,
        accounts: LoginAccounts,
        target: Target,
        ending: Ending,
    ) -> Result<(ProviderId, CredentialId), Unfinished> {
        let Ending {
            deadline,
            cancel,
            codes,
            held: _held,
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
        let download = {
            let flows = self.clone();
            let id = id.clone();
            move |progress: DownloadProgress| flows.download(&id, progress)
        };
        let accounts = accounts
            .accounts()
            .expect("a login's provider has account operations");
        let stop = CancellationToken::new();
        let login = accounts.login(LoginIo {
            pending: &pending,
            stage: &stage,
            download: &download,
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
            Ok(account) => self
                .publish(owner, target, account.id)
                .await
                .map_err(Unfinished::Failed),
            Err(LoginError::Stopped) if expired => {
                Err(Unfinished::Failed("The login expired".to_owned()))
            }
            Err(LoginError::Stopped) => Err(Unfinished::Cancelled),
            Err(error) => Err(Unfinished::Failed(error.to_string())),
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

    /// Stores how far the download the login's stage waits for came.
    fn download(&self, id: &LoginId, progress: DownloadProgress) {
        let mut flows = self.lock();
        let Some(flow) = flows.get_mut(id) else {
            return;
        };
        let LoginState::Pending {
            stage: LoginStage::Installing,
            download,
            ..
        } = &mut flow.state
        else {
            return;
        };
        if flow.cancel.is_cancelled() || *download == Some(progress) {
            return;
        }
        *download = Some(progress);
        let owner = flow.owner.clone();
        drop(flows);
        self.announce(&owner);
    }

    /// Stores what the vendor asked the user to do, while the login waits.
    ///
    /// The login still ends when it started to. A refusal of a code before the
    /// last one the login took is no longer news: the user pasted another
    /// since.
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
                tracing::info!(
                    event = "login.code_refused",
                    login = %id,
                    attempt = refused.code,
                    "the login's sign-in refused a code"
                );
                Some(refused.message)
            }
            Some(refused) => {
                tracing::info!(
                    event = "login.code_refused",
                    login = %id,
                    attempt = refused.code,
                    newer_attempt = codes_taken,
                    "the login's sign-in refused a code the user has pasted another after"
                );
                None
            }
            None => None,
        };
        let state = LoginState::Pending {
            stage: LoginStage::WaitingForUser,
            download: None,
            verification_url: Some(pending.verification_url),
            user_code: pending.user_code,
            needs_code: flow.kind == Some(LoginKind::PastedCode),
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

    /// Publishes the login's account, as a new entry with its staged accounts
    /// or into the existing entry.
    ///
    /// A concurrent login of the same family may have won the new entry. The
    /// existing entry's provider and catalog then start afresh.
    ///
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
                    .vault
                    .create_subscription(owner, family.clone(), label, &staged)
                    .await;
                match created {
                    Ok(Some(entry)) => entry.id,
                    Ok(None) => return Err(LoginRefusal::Exists(family).to_string()),
                    Err(error) => return Err(error.to_string()),
                }
            }
            Target::Existing { entry, .. } => {
                self.provider_registry
                    .invalidate(&entry.id)
                    .await
                    .map_err(|error| error.to_string())?;
                entry.id
            }
        };
        Ok((provider_id, account))
    }
}
