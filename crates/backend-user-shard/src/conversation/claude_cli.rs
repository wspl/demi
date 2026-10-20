//! Demi's Claude Code CLI on the Claude Code machine
//! (`crates/command-package-claude-code/src/lib.rs`,
//! `crates/provider-claude-code/src/placement.rs`,
//! `crates/provider-claude-code/src/account.rs`;
//! `crates/backend-cloud/src/lib.rs`).
//!
//! The module covers:
//!
//! - the versions the machine has and the install of the vendor's newest,
//!   through the `demi.claude-code` package;
//! - the placement that starts a provider's process there, in its account's
//!   directory with a directory of its own for its session, or a sign-in's new
//!   directory;
//! - the machine as every user's sign-in and usage probe reach it;
//! - the installs no conversation asked for, whose outcome the settings page
//!   shows.
//!
//! # Installed versions
//!
//! What each machine has installed is kept per runner connection, and a process
//! start asks the machine nothing: the root's process and three subagents'
//! starting on a laptop ask its runner once in all.
//!
//! - Showing a version is never a reason to wake a machine.
//! - A newer version does not interrupt work: a machine with a usable version
//!   keeps answering with it while the newer one installs beside it.
//! - Installing leaves nothing in the transcript. A failure is that request's
//!   error, or the result of the account action that asked.
//!
//! # The master's shard
//!
//! The machine's record is the master's, so the master's shard holds it. Another
//! user's work crosses there once per process start: the master's shard starts
//! the process, and a task there relays its standard streams and its end as
//! `Send` pipe ends. The task closes the process, SIGTERM first, once the user's
//! side lets go.
//!
//! Each run, sign-in and probe holds a lease of the machine for its whole life,
//! which keeps it awake and wakes it.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use bytes::Bytes;
use demi_backend_cloud::access::MachineAccess;
use demi_backend_database::control::ControlService;
use demi_backend_plugins::Lost;
use demi_backend_providers::llm::claude_releases::ReleaseError;
use demi_backend_remote_host::{
    AttachedArtifact, Link, RemoteHost, ServiceCallError, ServiceRequest, WeakLink,
};
use demi_backend_runners::command_context::{command_context, provider_context};
use demi_command_package_claude_code_protocol::{
    Installed, Operation, PACKAGE, Release, Reply, Status,
};
use demi_command_protocol::{
    ArtifactLocation, ArtifactUrl, CommandCaller, CommandContext, PackageArtifact,
};
use demi_host_interface::{
    FileContents, Host as _, HostError, HostErrorKind, MkdirOptions, Process, ProcessControl,
    ProcessEnd, ProcessOutput, RmOptions, Signal, SpawnError, SpawnErrorKind, WhenExists,
    WriteOptions,
};
use demi_provider_claude_code::{
    AccountMachine, AccountWork, CliSite, CliStart, CliStarter, CliSystem, CliWork, ConfigDir,
    Placed, Placement, PreparedProcesses, StartError, StartStage, StartSteps, WorkLease,
    close_process,
};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, ProviderId, UserId};
use demi_web_api_protocol::providers::CliInstall;
use futures_util::future::{BoxFuture, LocalBoxFuture, Shared};
use futures_util::{FutureExt as _, StreamExt as _};
use serde::de::DeserializeOwned;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::services::Services;
use crate::shard::{Shard, Shards};

/// The most an operation of the package answers: one JSON document.
const MAX_ANSWER_BYTES: usize = 1024 * 1024;

/// Why the machine has no usable CLI, with the version that was wanted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CliError {
    version: Option<String>,
    reason: String,
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.version {
            Some(version) => write!(
                f,
                "Claude Code {version} could not be installed: {}",
                self.reason
            ),
            None => write!(f, "Claude Code could not be installed: {}", self.reason),
        }
    }
}

impl std::error::Error for CliError {}

impl From<ReleaseError> for CliError {
    fn from(error: ReleaseError) -> Self {
        Self {
            version: None,
            reason: error.to_string(),
        }
    }
}

/// What a provider's process is for (`crates/provider-claude-code/src/placement.rs`); it
/// names the command context the package's operations run under.
#[derive(Debug, Clone)]
pub(crate) enum ProcessWork {
    /// A request of the conversation.
    Conversation(ConversationId),
    /// Work of the provider entry that belongs to no conversation: an
    /// install, or **Test connection**.
    Account(ProviderId),
}

/// The machine the CLI is wanted on: its device, a Host there, its home,
/// and the context the package's operations run under.
#[derive(Clone)]
struct CliTarget {
    device: DeviceId,
    host: RemoteHost,
    home: String,
    context: CommandContext,
}

/// The Claude Code machine as every shard reaches it: through the shard of
/// the instance's master, which holds it
/// (`crates/backend-cloud/src/lib.rs`). It serves a sign-in and a usage
/// probe from any thread.
#[derive(Clone)]
pub struct ClaudeMachine {
    shards: Shards,
    control: ControlService,
    /// The master, read once: an instance's master never changes.
    master: Arc<tokio::sync::OnceCell<UserId>>,
}

impl ClaudeMachine {
    pub fn new(shards: Shards, control: ControlService) -> Self {
        Self {
            shards,
            control,
            master: Arc::default(),
        }
    }

    async fn master(&self) -> Result<UserId, StartError> {
        self.master
            .get_or_try_init(|| async {
                self.control
                    .master()
                    .await
                    .map_err(|error| StartError(format!("The instance's master could not be read: {error}")))?
                    .ok_or_else(|| StartError("The instance has no master yet".into()))
            })
            .await
            .cloned()
    }

    /// Runs `work` on the master's shard, which holds the machine, and
    /// answers its result. Dropping the answer cancels the token `work`
    /// receives.
    async fn on_machine<F, Fut, T>(&self, work: F) -> Result<T, StartError>
    where
        F: FnOnce(Rc<Shard>, CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = T> + 'static,
        T: Send + 'static,
    {
        let master = self.master().await?;
        self.shards
            .of(&master)
            .call(work)
            .await
            .map_err(|error| StartError(error.to_string()))
    }
}

impl AccountMachine for ClaudeMachine {
    fn run(&self, entry: String, work: AccountWork) -> BoxFuture<'static, Result<(), StartError>> {
        let machine = self.clone();
        Box::pin(async move {
            let entry = ProviderId::try_from(entry).map_err(|error| StartError(error.to_string()))?;
            machine
                .on_machine(move |shard, abandoned| async move {
                    let control = &shard.services().control;
                    let context = match provider_context(control, shard.user(), &entry).await {
                        Ok(context) => context,
                        Err(error) => {
                            tracing::warn!(entry = %entry, "Claude Code account work found no command context: {error}");
                            return;
                        }
                    };
                    let placement = MachinePlacement::placement(Rc::downgrade(&shard), context);
                    work(placement, abandoned).await;
                })
                .await
        })
    }

    fn remove_account(&self, account: String) -> BoxFuture<'static, Result<(), StartError>> {
        let machine = self.clone();
        Box::pin(async move {
            machine
                .on_machine(move |shard, _| async move {
                    let access = shard
                        .cloud_shard()
                        .claude_code_access()
                        .await
                        .map_err(|error| StartError(error.to_string()))?;
                    let gates = shard.claude_cli().gates.clone();
                    gates.remove(&access, &account).await
                })
                .await?
        })
    }
}

/// The placement of a provider's process for a user's conversation or
/// entry: every start crosses once to the master's shard, which starts the
/// process on the Claude Code machine and relays it back as `Send` pipe
/// ends; the processes prepared ahead are the user's own.
pub(crate) struct CrossPlacement {
    shard: Weak<Shard>,
    work: ProcessWork,
    machine: ClaudeMachine,
    /// The processes prepared ahead for the user.
    prepared: Rc<PreparedProcesses>,
}

impl CrossPlacement {
    /// The placement a provider's process of `work` starts with.
    pub fn placement(shard: Weak<Shard>, work: ProcessWork) -> Rc<dyn Placement> {
        // A shard that is gone starts nothing, so what it would share
        // matters to nobody.
        let (prepared, machine) = match shard.upgrade() {
            Some(shard) => (shard.claude_cli().prepared.clone(), shard.claude_machine()),
            None => (Rc::default(), None),
        };
        match machine {
            Some(machine) => Rc::new(Self {
                shard,
                work,
                machine,
                prepared,
            }),
            None => Rc::new(Unplaced(prepared)),
        }
    }

    /// The command context the package's operations run under for the work.
    async fn context(&self, shard: &Shard) -> Result<CommandContext, StartError> {
        let control = &shard.services().control;
        let context = match &self.work {
            ProcessWork::Conversation(id) => {
                command_context(control, shard.user(), id, CommandCaller::User {}).await
            }
            ProcessWork::Account(provider) => provider_context(control, shard.user(), provider).await,
        };
        context.map_err(|error| StartError(format!("The command context could not be read: {error}")))
    }
}

impl Placement for CrossPlacement {
    fn start<'a>(
        &'a self,
        work: CliWork,
        start: CliStarter,
        stages: &'a dyn Fn(StartStage),
    ) -> LocalBoxFuture<'a, Result<Placed, StartError>> {
        Box::pin(async move {
            let shard = self
                .shard
                .upgrade()
                .ok_or_else(|| StartError("The backend is shutting down".into()))?;
            let context = self.context(&shard).await?;
            drop(shard);
            let (staged, mut reached) = mpsc::unbounded_channel();
            let started = self.machine.on_machine(move |shard, abandoned| async move {
                let placement = MachinePlacement::new(Rc::downgrade(&shard), context);
                let stages = move |stage| {
                    // A start nobody waits for reports to nobody.
                    let _ = staged.send(stage);
                };
                let placed = tokio::select! {
                    placed = placement.place(work, start, &stages) => placed,
                    () = abandoned.cancelled() => return Err(StartError("The start was given up".into())),
                };
                placed.map(|(placed, hold)| relay(&shard, placed, hold))
            });
            tokio::pin!(started);
            let relayed = loop {
                tokio::select! {
                    started = &mut started => break started,
                    Some(stage) = reached.recv() => stages(stage),
                }
            };
            while let Ok(stage) = reached.try_recv() {
                stages(stage);
            }
            let Relayed { pipes, steps } = relayed??;
            Ok(Placed {
                process: pipes.process(),
                config: Box::new(RelayedDir),
                steps,
            })
        })
    }

    fn work(&self) -> WorkLease {
        let (held, released) = oneshot::channel::<()>();
        let machine = self.machine.clone();
        if let Some(shard) = self.shard.upgrade() {
            shard.tasks().spawn_local(async move {
                let held = machine
                    .on_machine(move |shard, _| async move {
                        let lease = MachinePlacement::lease(&shard);
                        // The lease is held until the user's side lets go.
                        shard.tasks().spawn_local(async move {
                            let _lease = lease;
                            let _ = released.await;
                        });
                    })
                    .await;
                if let Err(error) = held {
                    tracing::warn!("the Claude Code machine was not held for work: {error}");
                }
            });
        }
        WorkLease::new(held)
    }

    fn prepared(&self) -> Rc<PreparedProcesses> {
        self.prepared.clone()
    }
}

/// The placement of a shard that has no Claude Code machine to reach, as in
/// a backend that has no shards: every start fails.
struct Unplaced(Rc<PreparedProcesses>);

impl Placement for Unplaced {
    fn start<'a>(
        &'a self,
        _: CliWork,
        _: CliStarter,
        _: &'a dyn Fn(StartStage),
    ) -> LocalBoxFuture<'a, Result<Placed, StartError>> {
        Box::pin(async { Err(StartError("The Claude Code machine is not available".into())) })
    }

    fn work(&self) -> WorkLease {
        WorkLease::new(())
    }

    fn prepared(&self) -> Rc<PreparedProcesses> {
        self.0.clone()
    }
}

/// A process the master's shard started and relays: its standard streams
/// and its end as `Send` pipe ends, and how long each step of its start
/// took.
struct Relayed {
    pipes: Pipes,
    steps: StartSteps,
}

/// What the relay's other end holds of a process: its output, its control
/// and its end.
struct Pipes {
    output: mpsc::UnboundedReceiver<ProcessOutput>,
    control: mpsc::UnboundedSender<Order>,
    exit: oneshot::Receiver<ProcessEnd>,
}

/// What the user's side asks of a relayed process.
enum Order {
    Write(Bytes, oneshot::Sender<Result<(), HostError>>),
    CloseStdin(oneshot::Sender<Result<(), HostError>>),
    Kill(Signal, oneshot::Sender<Result<(), HostError>>),
}

impl Pipes {
    /// The process as the user's side drives it.
    fn process(self) -> Process {
        let Self {
            output,
            control,
            exit,
        } = self;
        Process {
            output: futures_util::stream::unfold(output, |mut output| async move {
                output.recv().await.map(|chunk| (chunk, output))
            })
            .boxed_local(),
            control: Box::new(RelayedControl(control)),
            exit: async move {
                exit.await.unwrap_or_else(|_| {
                    ProcessEnd::Lost("the Claude Code machine's relay of the process ended".into())
                })
            }
            .boxed_local(),
        }
    }
}

/// The user's side of a relayed process's control; dropping it closes the
/// process on the machine, SIGTERM first.
struct RelayedControl(mpsc::UnboundedSender<Order>);

impl RelayedControl {
    async fn ask(&self, order: impl FnOnce(oneshot::Sender<Result<(), HostError>>) -> Order) -> Result<(), HostError> {
        let (answer, answered) = oneshot::channel();
        let gone = || HostError::new(HostErrorKind::Unavailable, "the process's relay ended");
        self.0.send(order(answer)).map_err(|_| gone())?;
        answered.await.unwrap_or_else(|_| Err(gone()))
    }
}

impl ProcessControl for RelayedControl {
    fn write_stdin(&self, bytes: Bytes) -> LocalBoxFuture<'_, Result<(), HostError>> {
        Box::pin(self.ask(|answer| Order::Write(bytes, answer)))
    }

    fn close_stdin(&self) -> LocalBoxFuture<'_, Result<(), HostError>> {
        Box::pin(self.ask(Order::CloseStdin))
    }

    fn kill(&self, signal: Signal) -> LocalBoxFuture<'_, Result<(), HostError>> {
        Box::pin(self.ask(move |answer| Order::Kill(signal, answer)))
    }
}

/// A relayed process's own directory, which the relay removes once the
/// process ended.
struct RelayedDir;

impl ConfigDir for RelayedDir {
    fn read<'a>(&'a self, name: &'a str) -> LocalBoxFuture<'a, Result<Bytes, HostError>> {
        Box::pin(async move {
            Err(HostError::new(
                HostErrorKind::Protocol,
                format!("{name} of a relayed process is not read"),
            ))
        })
    }

    fn remove(self: Box<Self>) -> LocalBoxFuture<'static, ()> {
        Box::pin(async {})
    }

    fn adopt<'a>(self: Box<Self>, _: &'a str) -> LocalBoxFuture<'a, Result<(), HostError>> {
        Box::pin(async {
            Err(HostError::new(
                HostErrorKind::Protocol,
                "a relayed process has no sign-in to adopt",
            ))
        })
    }
}

/// Starts relaying `placed`, a process the master's shard started, holding its
/// account's gate (`hold`), as a task of that shard.
///
/// - Its output goes to the pipe.
/// - The user's side's orders reach its control.
/// - Its end goes back once its directory is removed, after which the gate is
///   let go.
///
/// When the user's side lets go of the control, or a change of the account
/// begins, the process is closed, SIGTERM first. The task owns the process's
/// control, so no path drops it raw.
fn relay(shard: &Shard, placed: Placed, hold: Option<AccountHold>) -> Relayed {
    let Placed {
        process,
        config,
        steps,
    } = placed;
    let Process {
        mut output,
        control,
        exit,
    } = process;
    let (sent, received) = mpsc::unbounded_channel();
    let (orders, mut ordered) = mpsc::unbounded_channel::<Order>();
    let (ended, exited) = oneshot::channel();
    let exit = exit.shared();
    let closing = hold
        .as_ref()
        .map_or_else(CancellationToken::new, |hold| hold.closing.clone());
    shard.tasks().spawn_local(async move {
        let forwarding = async {
            while let Some(chunk) = output.next().await {
                // The user's side reads the output while it holds the
                // process; one that let go reads nothing more.
                let _ = sent.send(chunk);
            }
            exit.clone().await;
        };
        let obeying = async {
            while let Some(order) = ordered.recv().await {
                // An order whose asker gave up wants no answer.
                let _ = match order {
                    Order::Write(bytes, answer) => answer.send(control.write_stdin(bytes).await),
                    Order::CloseStdin(answer) => answer.send(control.close_stdin().await),
                    Order::Kill(signal, answer) => answer.send(control.kill(signal).await),
                };
            }
        };
        // Whether the process is to be closed: the user's side let go, or
        // a change of the account began; otherwise it ended.
        let close = tokio::select! {
            () = forwarding => false,
            () = obeying => true,
            () = closing.cancelled() => true,
        };
        if close {
            close_process(control, exit.clone()).await;
        }
        let end = exit.await;
        config.remove().await;
        drop(hold);
        // A user's side that let go learns nothing more.
        let _ = ended.send(end);
    });
    Relayed {
        pipes: Pipes {
            output: received,
            control: orders,
            exit: exited,
        },
        steps,
    }
}

/// The accounts whose processes run on the Claude Code machine, each with its
/// gate, which the master's shard keeps
/// (`crates/provider-claude-code/src/account.rs`).
///
/// Every process of an account holds its gate shared, from before its directory
/// is readied until the directory the process made of its own is removed.
///
/// Signing the account in again and removing it take the gate whole, one change
/// at a time:
///
/// 1. The change stops new starts of the account, which wait.
/// 2. It closes every process holding the gate and waits for them.
/// 3. Only then does it change the account's directory.
#[derive(Default)]
pub(crate) struct AccountGates(RefCell<HashMap<String, Rc<AccountGate>>>);

/// One account's gate.
#[derive(Default)]
struct AccountGate {
    /// Held shared by the account's processes, whole by a change. Tokio's
    /// lock queues in order, so once a change waits for it no start takes
    /// it before the change.
    lock: Arc<tokio::sync::RwLock<()>>,
    /// Fires as a change begins: every process holding the gate is closed.
    closing: RefCell<CancellationToken>,
    /// The account was removed: a start that waited fails.
    removed: Cell<bool>,
    /// One handover of an earlier release's sign-in at a time.
    handover: tokio::sync::Mutex<()>,
}

/// A process's hold of its account's gate.
struct AccountHold {
    gate: Rc<AccountGate>,
    _shared: tokio::sync::OwnedRwLockReadGuard<()>,
    /// Fires when a change of the account begins.
    closing: CancellationToken,
}

impl AccountGates {
    fn gate(&self, account: &str) -> Rc<AccountGate> {
        self.0
            .borrow_mut()
            .entry(account.to_owned())
            .or_default()
            .clone()
    }

    /// Holds `account`'s gate for a process, once no change of it is under
    /// way or waiting; a start that waited for the account's removal fails.
    async fn hold(&self, account: &str) -> Result<AccountHold, StartError> {
        let gate = self.gate(account);
        let shared = match gate.lock.clone().try_read_owned() {
            Ok(shared) => shared,
            Err(_) => {
                tracing::info!(account, "a start of a Claude Code account waits for a change of the account");
                gate.lock.clone().read_owned().await
            }
        };
        if gate.removed.get() {
            return Err(StartError("The Claude Code account was removed".into()));
        }
        let closing = gate.closing.borrow().clone();
        Ok(AccountHold {
            gate,
            _shared: shared,
            closing,
        })
    }

    /// Takes `account`'s gate whole for a change: new starts wait from the
    /// moment this is called, the account's processes are closed, SIGTERM
    /// first, and the gate is held once every one ended and its directory
    /// went. Dropping the future before that gives the change up.
    async fn whole(&self, account: &str) -> (Rc<AccountGate>, tokio::sync::OwnedRwLockWriteGuard<()>) {
        let gate = self.gate(account);
        let whole = gate.lock.clone().write_owned();
        tokio::pin!(whole);
        // The first poll queues the change, and a later one finds it
        // queued: Tokio's waiter keeps its place and takes the new waker.
        if let Some(held) = whole.as_mut().now_or_never() {
            return (gate, held);
        }
        gate.closing.replace(CancellationToken::new()).cancel();
        let held = whole.await;
        (gate, held)
    }

    /// Deletes the directory of the removed account `account` on the
    /// machine `access` reaches, once no process of it runs or starts;
    /// the starts that waited then fail. A deletion that fails leaves the
    /// account as it was.
    async fn remove(&self, access: &MachineAccess, account: &str) -> Result<(), StartError> {
        let directory = account_dir(&format!("{}/accounts", access.home), account)?;
        let (gate, _whole) = self.whole(account).await;
        let options = RmOptions {
            recursive: true,
            force: true,
        };
        access.host.fs().rm(&directory, options).await.map_err(|error| {
            StartError(format!("The account's directory could not be deleted on Claude Code's machine: {error}"))
        })?;
        gate.removed.set(true);
        Ok(())
    }
}

/// The Claude Code machine as the master's shard starts a process on it, for a
/// user's start that crossed there or for a sign-in or a usage probe that runs
/// there.
///
/// Each start:
///
/// 1. Wakes or admits the machine through its machine access.
/// 2. Makes sure the machine has the CLI.
/// 3. Makes the account's directory (handing an earlier release's sign-in over
///    into it once) and the process's own directory under it, or a sign-in's
///    new directory.
/// 4. Writes the start's files and starts the process.
///
/// It holds the machine's admission only until the process started; a process
/// that is retained holds none afterwards.
pub(crate) struct MachinePlacement {
    shard: Weak<Shard>,
    /// The context the package's operations run under.
    context: CommandContext,
}

impl MachinePlacement {
    pub(crate) fn new(shard: Weak<Shard>, context: CommandContext) -> Self {
        Self { shard, context }
    }

    pub(crate) fn placement(shard: Weak<Shard>, context: CommandContext) -> Rc<dyn Placement> {
        Rc::new(Self::new(shard, context))
    }

    /// A hold of the machine for work starting now, which wakes it when it
    /// is stopped, taken on the master's shard.
    fn lease(shard: &Rc<Shard>) -> WorkLease {
        let (held, mut released) = oneshot::channel::<()>();
        let holder = shard.clone();
        shard.tasks().spawn_local(async move {
            let admitted = tokio::select! {
                admitted = holder.cloud_shard().claude_code_access() => admitted,
                _ = &mut released => return,
            };
            match admitted {
                // The admission keeps the machine running until the work
                // lets go.
                Ok(_access) => {
                    let _ = released.await;
                }
                Err(error) => {
                    tracing::warn!("the Claude Code machine could not be held for work: {error}")
                }
            }
        });
        WorkLease::new(held)
    }
}

impl MachinePlacement {
    /// Starts a process for `work` on the machine, as [`Placement::start`]
    /// says, and answers it raw, with its hold of its account's gate for an
    /// account's process, which is held from before the account's directory
    /// is readied: whoever takes the process relays it with the hold.
    async fn place(
        &self,
        work: CliWork,
        start: CliStarter,
        stages: &dyn Fn(StartStage),
    ) -> Result<(Placed, Option<AccountHold>), StartError> {
        {
            let shard = self
                .shard
                .upgrade()
                .ok_or_else(|| StartError("The backend is shutting down".into()))?;
            stages(StartStage::Machine);
            // Each step's time, which the run's span carries
            // (`crates/provider-claude-code/src/prepared.rs`).
            let mut steps = StartSteps::default();
            let mut step = Instant::now();
            let mut lap = |into: &mut Duration| {
                let now = Instant::now();
                *into = now - step;
                step = now;
            };
            let access = shard
                .cloud_shard()
                .claude_code_access()
                .await
                .map_err(|error| StartError(error.to_string()))?;
            lap(&mut steps.admit);
            let executable = shard
                .cli_executable_at(&access, &self.context, stages)
                .await
                .map_err(StartError)?;
            lap(&mut steps.executable);
            stages(StartStage::Process);
            let fs = access.host.fs();
            let made = |what: &str, error: HostError| {
                StartError(format!("Claude Code's {what} could not be made on its machine: {error}"))
            };
            let run_dir = format!("{}/run", access.home);
            fs.mkdir(&run_dir, MkdirOptions { recursive: true })
                .await
                .map_err(|error| made("run directory", error))?;
            let accounts = format!("{}/accounts", access.home);
            fs.mkdir(&accounts, MkdirOptions { recursive: true })
                .await
                .map_err(|error| made("directory of accounts", error))?;
            // Dropping the directory's handle before the process started,
            // as when the start is given up, removes what it made.
            let (config_dir, process_dir, mut dir, hold) = match work {
                CliWork::Account { account, handover } => {
                    let account_dir = account_dir(&accounts, &account)?;
                    // A change of the account under way or waiting goes
                    // first; this start then finds its directory as the
                    // change left it.
                    let gates = shard.claude_cli().gates.clone();
                    let hold = gates.hold(&account).await?;
                    fs.mkdir(&account_dir, MkdirOptions { recursive: true })
                        .await
                        .map_err(|error| made("account's directory", error))?;
                    fs.chmod(&account_dir, 0o700)
                        .await
                        .map_err(|error| made("account's directory", error))?;
                    if let Some(handover) = handover {
                        hand_over(&access.host, &account_dir, handover.files, &hold.gate)
                            .await
                            .map_err(|error| made("handed-over sign-in", error))?;
                    }
                    let name = uuid::Uuid::new_v4().simple().to_string();
                    let path = format!("{account_dir}/projects/{name}");
                    fs.mkdir(&path, MkdirOptions { recursive: true })
                        .await
                        .map_err(|error| made("process's directory", error))?;
                    let dir = MachineDir::new(&self.shard, &access.host, path, None);
                    (account_dir, name, dir, Some(hold))
                }
                CliWork::Login => {
                    let path = format!("{accounts}/.login-{}", uuid::Uuid::new_v4().simple());
                    fs.mkdir(&path, MkdirOptions { recursive: false })
                        .await
                        .map_err(|error| made("sign-in's directory", error))?;
                    fs.chmod(&path, 0o700)
                        .await
                        .map_err(|error| made("sign-in's directory", error))?;
                    let dir = MachineDir::new(&self.shard, &access.host, path.clone(), Some(accounts));
                    (path, "demi".to_owned(), dir, None)
                }
            };
            lap(&mut steps.directories);
            let system = shard.cli_system(&access, &run_dir).await.map_err(|error| {
                StartError(format!(
                    "The Claude Code machine could not be asked what it tells Claude Code of itself: {error}"
                ))
            })?;
            lap(&mut steps.system);
            let site = CliSite {
                executable,
                run_dir,
                config_dir,
                process_dir,
                system,
            };
            // The session's bytes are made here, for the machine's system,
            // and written next: one step.
            let CliStart { spawn, files } = start(&site);
            steps.file_bytes = files.iter().map(|(_, bytes)| bytes.len() as u64).sum();
            dir.write(files).await.map_err(|error| {
                StartError(format!(
                    "Claude Code's session could not be written on its machine: {error}"
                ))
            })?;
            lap(&mut steps.files);
            let process = access
                .host
                .process()
                .spawn(spawn)
                .await
                .map_err(|error| {
                    StartError(format!(
                        "Claude Code could not be started on its machine: {error}"
                    ))
                })?;
            lap(&mut steps.spawn);
            // A start that finds the executable it was given gone, as after
            // another runner sharing the cache removed it, drops what is
            // kept of the machine's versions
            // (`crates/backend-providers/src/llm/claude_releases.rs`).
            let kept_by = self.shard.clone();
            let device = access.device.id.clone();
            let exit = process
                .exit
                .map(move |end| {
                    let gone = matches!(
                        &end,
                        ProcessEnd::NotStarted(SpawnError {
                            kind: SpawnErrorKind::ExecutableNotFound,
                            ..
                        })
                    );
                    // A shard that is gone keeps nothing to drop.
                    if let (true, Some(shard)) = (gone, kept_by.upgrade()) {
                        shard.claude_cli().forget_versions(&device);
                        tracing::info!(device = %device, "the Claude Code versions kept for the device were dropped: its executable was gone");
                    }
                    end
                })
                .boxed_local()
                .shared();
            dir.exit = Some(exit.clone());
            let placed = Placed {
                process: Process {
                    exit: exit.boxed_local(),
                    ..process
                },
                config: Box::new(dir),
                steps,
            };
            Ok((placed, hold))
        }
    }
}

impl Placement for MachinePlacement {
    /// An account's process, a usage probe's, is relayed by a task of the
    /// shard, which owns its control and closes it, SIGTERM first, when the
    /// work lets go or a change of the account begins; a sign-in's is the
    /// sign-in's own.
    fn start<'a>(
        &'a self,
        work: CliWork,
        start: CliStarter,
        stages: &'a dyn Fn(StartStage),
    ) -> LocalBoxFuture<'a, Result<Placed, StartError>> {
        Box::pin(async move {
            let (placed, hold) = self.place(work, start, stages).await?;
            let Some(hold) = hold else {
                return Ok(placed);
            };
            let shard = self
                .shard
                .upgrade()
                .ok_or_else(|| StartError("The backend is shutting down".into()))?;
            let Relayed { pipes, steps } = relay(&shard, placed, Some(hold));
            Ok(Placed {
                process: pipes.process(),
                config: Box::new(RelayedDir),
                steps,
            })
        })
    }

    fn work(&self) -> WorkLease {
        match self.shard.upgrade() {
            Some(shard) => Self::lease(&shard),
            None => WorkLease::new(()),
        }
    }

    fn prepared(&self) -> Rc<PreparedProcesses> {
        // A sign-in or a probe prepares nothing ahead.
        Rc::default()
    }
}

/// The directory of the account `account` among `accounts`: its id names
/// it, and an id that is not a plain name names none.
fn account_dir(accounts: &str, account: &str) -> Result<String, StartError> {
    let plain = !account.is_empty()
        && !account.starts_with('.')
        && account
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));
    if !plain {
        return Err(StartError(format!("The account id {account} names no directory")));
    }
    Ok(format!("{accounts}/{account}"))
}

/// Writes the sign-in an earlier release kept, as the CLI's `files`, into the
/// account's directory `dir`, private to the machine's user, unless the
/// directory holds a sign-in already.
///
/// The directory holds a sign-in when its last file, the CLI's credentials, is
/// there, and nothing is written over it
/// (`crates/provider-claude-code/src/account.rs`). One handover of the account
/// runs at a time (`gate`).
///
/// Each file is written whole beside its place, then all are renamed into place
/// in order, so the directory holds a sign-in only once every file is there. A
/// handover that fails leaves no sign-in, and the record keeps it.
async fn hand_over(
    host: &RemoteHost,
    dir: &str,
    files: Vec<(String, Bytes)>,
    gate: &AccountGate,
) -> Result<(), HostError> {
    let _one = gate.handover.lock().await;
    let fs = host.fs();
    let Some((sign_in, _)) = files.last() else {
        return Ok(());
    };
    if fs.exists(&format!("{dir}/{sign_in}")).await? {
        return Ok(());
    }
    let stamp = uuid::Uuid::new_v4().simple().to_string();
    let placed: Vec<(String, String)> = files
        .iter()
        .map(|(name, _)| (format!("{dir}/{name}.handover-{stamp}"), format!("{dir}/{name}")))
        .collect();
    let written = async {
        for ((temporary, _), (_, bytes)) in placed.iter().zip(files) {
            let options = WriteOptions {
                exists: WhenExists::Replace,
            };
            fs.write_file(temporary, FileContents::Bytes(bytes), options).await?;
            fs.chmod(temporary, 0o600).await?;
        }
        for (temporary, path) in &placed {
            fs.mv(temporary, path).await?;
        }
        Ok(())
    }
    .await;
    if written.is_err() {
        let options = RmOptions {
            recursive: false,
            force: true,
        };
        for (temporary, _) in &placed {
            // What was renamed into place is gone from here; what is left
            // is a temporary, which the next handover writes anew.
            let _ = fs.rm(temporary, options).await;
        }
    }
    written
}

/// What a process made of its own on the Claude Code machine: an account's
/// process, its directory under the account's, removed once the process
/// ended; a sign-in, its new directory, removed the same way unless it
/// becomes an account's.
struct MachineDir {
    shard: Weak<Shard>,
    host: RemoteHost,
    path: String,
    /// For a sign-in, the directory of accounts its directory joins when
    /// it succeeds.
    accounts: Option<String>,
    /// The process's end; none until it started.
    exit: Option<Shared<LocalBoxFuture<'static, ProcessEnd>>>,
    removed: bool,
}

impl MachineDir {
    fn new(shard: &Weak<Shard>, host: &RemoteHost, path: String, accounts: Option<String>) -> Self {
        Self {
            shard: shard.clone(),
            host: host.clone(),
            path,
            accounts,
            exit: None,
            removed: false,
        }
    }

    /// Writes `files` into the directory, each at its path there.
    async fn write(&self, files: Vec<(String, Bytes)>) -> Result<(), HostError> {
        let fs = self.host.fs();
        let paths = self.host.identity().paths;
        for (name, bytes) in files {
            // `name` is a path in the directory, written with `/`.
            let path = paths.join(&self.path, &name);
            if let Some((parent, _)) = name.rsplit_once('/') {
                fs.mkdir(&paths.join(&self.path, parent), MkdirOptions { recursive: true })
                    .await?;
            }
            fs.write_file(&path, FileContents::Bytes(bytes), WriteOptions::default())
                .await?;
        }
        Ok(())
    }

    /// Removes the directory once its process ended.
    fn removal(&mut self) -> LocalBoxFuture<'static, ()> {
        self.removed = true;
        let host = self.host.clone();
        let path = self.path.clone();
        let exit = self.exit.take();
        Box::pin(async move {
            if let Some(exit) = exit {
                exit.await;
            }
            let options = RmOptions {
                recursive: true,
                force: true,
            };
            // A directory left behind is said here; the next sign-in or
            // process makes its own all the same.
            if let Err(error) = host.fs().rm(&path, options).await {
                tracing::warn!(path, "a Claude Code directory was not removed: {error}");
            }
        })
    }
}

impl ConfigDir for MachineDir {
    fn read<'a>(&'a self, name: &'a str) -> LocalBoxFuture<'a, Result<Bytes, HostError>> {
        Box::pin(async move { self.host.fs().read_file(&format!("{}/{name}", self.path)).await })
    }

    fn remove(mut self: Box<Self>) -> LocalBoxFuture<'static, ()> {
        self.removal()
    }

    fn adopt<'a>(mut self: Box<Self>, account: &'a str) -> LocalBoxFuture<'a, Result<(), HostError>> {
        Box::pin(async move {
            let Some(accounts) = self.accounts.clone() else {
                return Err(HostError::new(
                    HostErrorKind::Protocol,
                    "only a sign-in's directory becomes an account's",
                ));
            };
            let target = account_dir(&accounts, account)
                .map_err(|error| HostError::new(HostErrorKind::Protocol, error.0))?;
            if let Some(exit) = self.exit.take() {
                exit.await;
            }
            let shutting_down = || HostError::new(HostErrorKind::Unavailable, "the backend is shutting down");
            let gates = match self.shard.upgrade() {
                Some(shard) => shard.claude_cli().gates.clone(),
                None => return Err(shutting_down()),
            };
            // Waiting here for the account's processes gives up with the
            // adoption, and the sign-in's directory then goes.
            let (gate, whole) = gates.whole(account).await;
            let Some(shard) = self.shard.upgrade() else {
                return Err(shutting_down());
            };
            // From here the change runs to its end as a task of the shard,
            // even when the adoption is dropped: one cut between its steps
            // would leave the account without its directory.
            self.removed = true;
            let host = self.host.clone();
            let path = self.path.clone();
            let (done, changed) = oneshot::channel();
            shard.tasks().spawn_local(async move {
                let fs = host.fs();
                let options = RmOptions {
                    recursive: true,
                    force: true,
                };
                let replaced = async {
                    fs.rm(&target, options).await?;
                    fs.mv(&path, &target).await
                }
                .await;
                match &replaced {
                    // An account signed in anew after its removal, under
                    // the same id, starts processes again.
                    Ok(()) => gate.removed.set(false),
                    // A sign-in that did not become the account's goes.
                    Err(_) => {
                        let _ = fs.rm(&path, options).await;
                    }
                }
                drop(whole);
                // An adoption given up wants no answer.
                let _ = done.send(replaced);
            });
            changed.await.unwrap_or_else(|_| Err(shutting_down()))
        })
    }
}

impl Drop for MachineDir {
    /// A directory nobody removed or adopted goes once its process ended,
    /// as a task of the shard, which shutdown waits for.
    fn drop(&mut self) {
        if self.removed {
            return;
        }
        let removal = self.removal();
        if let Some(shard) = self.shard.upgrade() {
            shard.tasks().spawn_local(removal);
        }
    }
}

/// The CLI work of one user's shard.
///
/// It keeps:
///
/// - the versions each machine has;
/// - the versions being installed beside the one in use;
/// - the processes prepared ahead for the user;
/// - on the master's shard, what it keeps of the Claude Code machine: the run
///   directory's real path there and the gates of the accounts whose processes
///   it starts.
#[derive(Default)]
pub(crate) struct ClaudeCli {
    versions: RefCell<HashMap<DeviceId, KeptVersions>>,
    upgrading: RefCell<HashSet<String>>,
    prepared: Rc<PreparedProcesses>,
    /// The accounts whose processes the master's shard starts, each with
    /// its gate.
    gates: Rc<AccountGates>,
    /// The run directory with its links resolved, and the runner
    /// connection it was resolved over: a new connection resolves it again.
    run_dir: RefCell<Option<(WeakLink, String)>>,
}

/// The versions a machine has, newest install first, as one answer that
/// every start waiting for it shares.
type Versions = Shared<LocalBoxFuture<'static, Result<Vec<Installed>, CliError>>>;

/// What the backend keeps of the CLI versions a machine has
/// (`crates/backend-providers/src/llm/claude_releases.rs`).
///
/// It is the answer of the runner connection `link`, asked once on it, or what
/// the backend's own install over it left. A new connection asks again.
/// `waiting` counts the starts that wait for an ask still under way.
struct KeptVersions {
    link: WeakLink,
    versions: Versions,
    waiting: Rc<Cell<usize>>,
}

impl KeptVersions {
    fn new(link: WeakLink, versions: Versions) -> Self {
        Self {
            link,
            versions,
            waiting: Rc::default(),
        }
    }
}

/// A start waiting for an ask of a machine's versions. The last one to
/// give up before the answer drops the ask, so the next start asks again
/// rather than wait for an ask nobody waits for
/// (`crates/backend-providers/src/llm/claude_releases.rs`).
struct WaitingForVersions<'a> {
    cli: &'a ClaudeCli,
    device: &'a DeviceId,
    versions: Versions,
    waiting: Rc<Cell<usize>>,
}

impl<'a> WaitingForVersions<'a> {
    fn new(cli: &'a ClaudeCli, device: &'a DeviceId, versions: Versions, waiting: Rc<Cell<usize>>) -> Self {
        waiting.set(waiting.get() + 1);
        Self {
            cli,
            device,
            versions,
            waiting,
        }
    }
}

impl Drop for WaitingForVersions<'_> {
    fn drop(&mut self) {
        self.waiting.set(self.waiting.get() - 1);
        if self.waiting.get() > 0 || self.versions.peek().is_some() {
            return;
        }
        let mut kept = self.cli.versions.borrow_mut();
        if kept
            .get(self.device)
            .is_some_and(|kept| kept.versions.ptr_eq(&self.versions))
        {
            // Dropping the ask cancels its call to the machine.
            kept.remove(self.device);
            tracing::info!(device = %self.device, "the ask of the device's Claude Code versions was given up by every start that waited for it");
        }
    }
}

impl ClaudeCli {
    /// Drops what is kept of `device`'s versions, for its next start to ask
    /// again.
    fn forget_versions(&self, device: &DeviceId) {
        self.versions.borrow_mut().remove(device);
    }
}

impl Shard {
    /// What the machine `access` holds tells a process in `run_dir` of itself, as
    /// the CLI reads it to describe its environment
    /// (`crates/provider-claude-code/src/session.rs`).
    ///
    /// It gives the kernel and the `SHELL` its runner reported in its hello, and
    /// the directory with its links resolved, once per runner connection. No
    /// process starts for them.
    async fn cli_system(&self, access: &MachineAccess, run_dir: &str) -> Result<CliSystem, String> {
        let link = self
            .devices()
            .link(&access.device.id)
            .ok_or("the machine's runner is not connected")?;
        let kernel = link
            .kernel()
            .ok_or("the machine's runner reported no kernel")?
            .clone();
        let kept = self
            .claude_cli()
            .run_dir
            .borrow()
            .as_ref()
            .filter(|(over, _)| over.is(&link))
            .map(|(_, path)| path.clone());
        let working_directory = match kept {
            Some(path) => path,
            None => {
                let path = access
                    .host
                    .fs()
                    .realpath(run_dir)
                    .await
                    .map_err(|error| error.to_string())?;
                *self.claude_cli().run_dir.borrow_mut() = Some((link.downgrade(), path.clone()));
                path
            }
        };
        Ok(CliSystem {
            kernel: kernel.name,
            release: kernel.release,
            shell: link.shell().map(str::to_owned),
            working_directory,
        })
    }

    /// The CLI's executable on the machine `access` holds, installed first
    /// when the machine has none, which it reports to `stages`.
    async fn cli_executable_at(
        &self,
        access: &MachineAccess,
        context: &CommandContext,
        stages: &dyn Fn(StartStage),
    ) -> Result<String, String> {
        let target = CliTarget {
            device: access.device.id.clone(),
            host: access.host.clone(),
            home: access.home.clone(),
            context: context.clone(),
        };
        self.cli_executable(&target, stages)
            .await
            .map_err(|error| error.to_string())
    }

    /// The executable to start on `target`
    /// (`crates/backend-providers/src/llm/claude_releases.rs`): the wanted
    /// version when the machine has it, else the newest it has while the wanted
    /// one installs beside it.
    ///
    /// A machine with a version never waits for the release pointer: without a
    /// believed answer it starts its newest, and the pointer is read in the
    /// background for a later start. Only a machine with none waits for the read
    /// and the install, which it reports to `stages`.
    async fn cli_executable(
        &self,
        target: &CliTarget,
        stages: &dyn Fn(StartStage),
    ) -> Result<String, CliError> {
        let releases = &self.services().claude_releases;
        let installed = self.cli_versions(target).await?;
        let Some(newest) = installed.first() else {
            stages(StartStage::Install);
            let wanted = match releases.latest(false).await {
                Ok(wanted) => wanted,
                Err(error) => {
                    // The install failed before the machine was asked to
                    // do anything; what it has is asked again all the same.
                    self.claude_cli().forget_versions(&target.device);
                    return Err(error.into());
                }
            };
            let installed = self.ensure_cli(target, &wanted).await?;
            return Ok(installed.path.to_string_lossy().into_owned());
        };
        let Some(wanted) = releases.believed() else {
            return Ok(newest.path.to_string_lossy().into_owned());
        };
        let usable = installed
            .iter()
            .find(|installed| installed.version == wanted.version)
            .unwrap_or(newest);
        if usable.version != wanted.version {
            self.upgrade_cli(target.clone(), wanted);
        }
        Ok(usable.path.to_string_lossy().into_owned())
    }

    /// The versions `target`'s machine has, newest install first, as the backend
    /// keeps them for its runner connection
    /// (`crates/backend-providers/src/llm/claude_releases.rs`).
    ///
    /// The versions are asked once on the connection, by whichever start comes
    /// first, while the others wait for the same answer. A failed ask, or one
    /// every start waiting for it gave up on, is kept by nobody, so the next
    /// start asks again. A machine whose runner is not connected is asked and
    /// nothing is kept.
    async fn cli_versions(&self, target: &CliTarget) -> Result<Vec<Installed>, CliError> {
        let Some(link) = self.devices().link(&target.device) else {
            return installed(self.services(), target).await;
        };
        let (versions, waiting) = self.kept_or_asked(target, &link);
        let _waiting =
            WaitingForVersions::new(self.claude_cli(), &target.device, versions.clone(), waiting);
        let answer = versions.clone().await;
        if answer.is_err() {
            let mut kept = self.claude_cli().versions.borrow_mut();
            if kept
                .get(&target.device)
                .is_some_and(|kept| kept.versions.ptr_eq(&versions))
            {
                kept.remove(&target.device);
            }
        }
        answer
    }

    /// What is kept of `target`'s versions for the connection `link`, else
    /// an ask of the machine, which is kept from now on, with the count of
    /// the starts waiting for it. The ask runs while a start waits for it:
    /// the starts that wait poll it in turn.
    fn kept_or_asked(&self, target: &CliTarget, link: &Link) -> (Versions, Rc<Cell<usize>>) {
        let mut kept = self.claude_cli().versions.borrow_mut();
        if let Some(kept) = kept.get(&target.device).filter(|kept| kept.link.is(link)) {
            return (kept.versions.clone(), kept.waiting.clone());
        }
        // The ask holds the shard weakly: the shard keeps the ask.
        let shard = Rc::downgrade(&self.this());
        let asked = target.clone();
        let versions = async move {
            let Some(shard) = shard.upgrade() else {
                return Err(CliError {
                    version: None,
                    reason: "the backend is shutting down".into(),
                });
            };
            installed(shard.services(), &asked).await
        }
        .boxed_local()
        .shared();
        let entry = KeptVersions::new(link.downgrade(), versions.clone());
        let waiting = entry.waiting.clone();
        kept.insert(target.device.clone(), entry);
        (versions, waiting)
    }

    /// Installs `release` on `target` and keeps what the install left for
    /// the runner connection it ran over: that version alone, since the
    /// runner removes the others it does not hold (`crates/runner-command-packages/src/cache.rs`). A failed install drops what is kept, for the next start
    /// to ask the machine again.
    async fn ensure_cli(&self, target: &CliTarget, release: &Release) -> Result<Installed, CliError> {
        let link = self.devices().link(&target.device).map(|link| link.downgrade());
        let ensured = ensure(self.services(), target, release).await;
        let installed = match ensured {
            Ok(installed) => installed,
            Err(error) => {
                self.claude_cli().forget_versions(&target.device);
                return Err(error);
            }
        };
        let current = self.devices().link(&target.device);
        match (link, current) {
            (Some(link), Some(current)) if link.is(&current) => {
                let left: Result<_, CliError> = Ok(vec![installed.clone()]);
                let versions = futures_util::future::ready(left).boxed_local().shared();
                self.claude_cli()
                    .versions
                    .borrow_mut()
                    .insert(target.device.clone(), KeptVersions::new(link, versions));
            }
            // The connection ended meanwhile: the next one asks.
            _ => self.claude_cli().forget_versions(&target.device),
        }
        Ok(installed)
    }

    /// Drops what is kept of `device`'s CLI versions when `lost` names the
    /// package's state there, which went without the runner saying so
    /// (`crates/backend-providers/src/llm/claude_releases.rs`).
    pub(crate) fn cli_package_lost(&self, device: &DeviceId, lost: &Lost<'_>) {
        if matches!(lost, Lost::Every | Lost::Package(PACKAGE)) {
            self.claude_cli().forget_versions(device);
            tracing::info!(device = %device, "the Claude Code versions kept for the device were dropped: its package was lost");
        }
    }

    /// Installs `release` on `target` beside the version in use, once per version
    /// whoever noticed it first.
    ///
    /// Nothing waits for it: a failed update leaves the version in use, and the
    /// next start notices the newer one again. Its Host admits each operation
    /// without waking the machine, and shutdown cancels it.
    fn upgrade_cli(&self, target: CliTarget, release: Release) {
        if !self
            .claude_cli()
            .upgrading
            .borrow_mut()
            .insert(release.version.clone())
        {
            return;
        }
        let shard = self.this();
        self.tasks().spawn_local(async move {
            let upgraded = tokio::select! {
                upgraded = shard.ensure_cli(&target, &release) => Some(upgraded),
                () = shard.closed() => None,
            };
            if let Some(Err(error)) = upgraded {
                tracing::warn!(user = %shard.user(), "the Claude Code machine's CLI was not updated: {error}");
            }
            shard.claude_cli().upgrading.borrow_mut().remove(&release.version);
        });
    }

    /// Resolves once `count` processes began to be prepared for the user.
    #[cfg(feature = "testing")]
    pub async fn until_prepared(&self, count: u64) {
        self.claude_cli().prepared.until_begun(count).await;
    }

    /// Follows the vault's announcements of active-account changes until the
    /// shard closes (`crates/provider-claude-code/src/prepared.rs`).
    ///
    /// For each change, the processes prepared for the user in the previous
    /// account's directory close, and the conversations on the entry that a page
    /// has open prepare again. `changes` was subscribed when the shard was made,
    /// so no change since is missed; a follower that fell behind treats every
    /// entry as changed.
    pub(crate) async fn follow_account_changes(
        self: Rc<Self>,
        mut changes: tokio::sync::broadcast::Receiver<ProviderId>,
    ) {
        loop {
            let changed = tokio::select! {
                changed = changes.recv() => changed,
                () = self.closed() => return,
            };
            match changed {
                Ok(entry) => {
                    self.claude_cli().prepared.close_entry(entry.as_str());
                    self.agent().prepare_open(Some(entry.as_str()));
                }
                Err(RecvError::Lagged(missed)) => {
                    tracing::warn!(
                        user = %self.user(),
                        missed,
                        "account changes were missed; every prepared process is closed"
                    );
                    self.claude_cli().prepared.close_all();
                    self.agent().prepare_open(None);
                }
                // The vault lives as long as the backend.
                Err(RecvError::Closed) => return,
            }
        }
    }

    /// Installs the CLI on the Claude Code machine for account work of
    /// `entry`, as a task of the shard, and records the outcome for the
    /// settings page. Shutdown cancels it.
    pub(crate) fn install_cli(&self, entry: ProviderId) {
        let shard = self.this();
        self.tasks().spawn_local(async move {
            let installed = tokio::select! {
                installed = shard.install_cli_now(&entry) => installed,
                () = shard.closed() => return,
            };
            let outcome = match installed {
                Ok(path) => CliInstall::Installed { path },
                Err(message) => CliInstall::Failed { message },
            };
            shard
                .services()
                .cli_installs
                .finish(shard.user(), &entry, outcome);
        });
    }

    /// Installs the CLI on the Claude Code machine, on the master's shard,
    /// which holds it.
    async fn install_cli_now(&self, entry: &ProviderId) -> Result<String, String> {
        let machine = self
            .claude_machine()
            .ok_or("The Claude Code machine is not available")?;
        let context = provider_context(&self.services().control, self.user(), entry)
            .await
            .map_err(|error| format!("The command context could not be read: {error}"))?;
        machine
            .on_machine(move |shard, _| async move {
                let access = shard
                    .cloud_shard()
                    .claude_code_access()
                    .await
                    .map_err(|error| error.to_string())?;
                shard.cli_executable_at(&access, &context, &|_| {}).await
            })
            .await
            .map_err(|error| error.0)?
    }
}

/// The versions `target` has, newest first.
async fn installed(services: &Services, target: &CliTarget) -> Result<Vec<Installed>, CliError> {
    let status: Status = call(
        services,
        target,
        Operation::Status,
        Bytes::new(),
        Vec::new(),
        None,
    )
    .await?;
    Ok(status.installed)
}

/// Installs `release` on `target` and answers the installed executable.
async fn ensure(
    services: &Services,
    target: &CliTarget,
    release: &Release,
) -> Result<Installed, CliError> {
    let failed = |reason: String| CliError {
        version: Some(release.version.clone()),
        reason,
    };
    let record = serde_json::to_vec(release).map_err(|error| failed(error.to_string()))?;
    // The runner may download each of the release's executables from its
    // official URL (`crates/command-package-claude-code/src/lib.rs`).
    let attached = release
        .platforms
        .values()
        .map(|artifact| AttachedArtifact {
            artifact: PackageArtifact {
                sha256: artifact.sha256.clone(),
                size: artifact.size,
            },
            location: ArtifactLocation::Url(ArtifactUrl {
                url: artifact.url.clone(),
                expires_at: None,
            }),
        })
        .collect();
    call(
        services,
        target,
        Operation::Ensure,
        Bytes::from(record),
        attached,
        Some(&release.version),
    )
    .await
}

/// Runs `operation` of the package on `target` with `input` as its whole
/// input and `attached` as the artifacts it may install, and decodes the one
/// document it answers. A nonzero exit keeps a failure document; an output
/// with no document is a service that failed.
async fn call<T: DeserializeOwned>(
    services: &Services,
    target: &CliTarget,
    operation: Operation,
    input: Bytes,
    attached: Vec<AttachedArtifact>,
    version: Option<&str>,
) -> Result<T, CliError> {
    let failed = |reason: String| CliError {
        version: version.map(str::to_owned),
        reason,
    };
    let served = Operation::ALL.map(Operation::name);
    let Some(package) = services
        .native
        .package(PACKAGE)
        .filter(|_| services.native.serves(PACKAGE, &served))
    else {
        return Err(failed(
            "this deployment does not carry the demi.claude-code package, which installs it".into(),
        ));
    };
    let request = ServiceRequest {
        context: target.context.clone(),
        package: package.clone(),
        operation: operation.name().to_owned(),
        args: None,
        json: None,
        cwd: target.home.clone(),
        resolver: services.native.resolver(&services.public_url),
        attached,
    };
    let (output, exit) = match target
        .host
        .call_service(request, input, MAX_ANSWER_BYTES)
        .await
    {
        Ok(output) => (output, None),
        Err(ServiceCallError::Exited {
            exit_code,
            stderr,
            stdout,
        }) => (stdout, Some((exit_code, stderr))),
        Err(error) => return Err(failed(format!("the installer failed: {error}"))),
    };
    if output.iter().all(u8::is_ascii_whitespace) {
        let reason = match exit {
            Some((code, stderr)) => format!("the installer exited with {code}: {}", stderr.trim()),
            None => "the installer gave no answer".into(),
        };
        return Err(failed(reason));
    }
    match (serde_json::from_slice::<Reply<T>>(&output), exit) {
        (Ok(Reply::Failed(failure)), _) => Err(failed(failure.message)),
        (Ok(Reply::Done(answer)), None) => Ok(answer),
        (Ok(Reply::Done(_)), Some((code, _))) => Err(failed(format!(
            "the installer answered and exited with {code}"
        ))),
        (Err(error), _) => Err(failed(format!(
            "the installer's answer cannot be read: {error}"
        ))),
    }
}

/// The outcome of each install that no conversation asked for, by user and
/// entry, kept while the backend runs. A `std` mutex: no section awaits.
#[derive(Default)]
pub struct CliInstalls {
    states: Mutex<HashMap<(UserId, ProviderId), CliInstall>>,
}

impl CliInstalls {
    fn lock(&self) -> MutexGuard<'_, HashMap<(UserId, ProviderId), CliInstall>> {
        // A section only reads or replaces states.
        self.states.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The last install `user` started of `entry`'s CLI.
    pub fn state(&self, user: &UserId, entry: &ProviderId) -> Option<CliInstall> {
        self.lock().get(&(user.clone(), entry.clone())).cloned()
    }

    /// Records an install as started; false when one is under way.
    fn begin(&self, user: &UserId, entry: &ProviderId) -> bool {
        let mut states = self.lock();
        let key = (user.clone(), entry.clone());
        if matches!(states.get(&key), Some(CliInstall::Installing {})) {
            return false;
        }
        states.insert(key, CliInstall::Installing {});
        true
    }

    /// Records how an install ended, unless its entry was deleted meanwhile.
    fn finish(&self, user: &UserId, entry: &ProviderId, outcome: CliInstall) {
        if let Some(state) = self.lock().get_mut(&(user.clone(), entry.clone())) {
            *state = outcome;
        }
    }

    /// Forgets the installs of a deleted entry.
    pub fn forget(&self, entry: &ProviderId) {
        self.lock().retain(|(_, installed), _| installed != entry);
    }
}

/// Starts the install of `entry`'s CLI on the Claude Code machine for
/// `user` unless one is
/// under way, and answers its state. Nobody waits for it.
pub async fn start_install(
    services: &Services,
    shards: &Shards,
    user: &UserId,
    entry: &ProviderId,
) -> CliInstall {
    if !services.cli_installs.begin(user, entry) {
        return CliInstall::Installing {};
    }
    let id = entry.clone();
    let queued = shards
        .of(user)
        .call(move |shard, _| async move { shard.install_cli(id) })
        .await;
    match queued {
        Ok(()) => CliInstall::Installing {},
        Err(error) => {
            let failed = CliInstall::Failed {
                message: error.to_string(),
            };
            services.cli_installs.finish(user, entry, failed.clone());
            failed
        }
    }
}
