//! Where the CLI runs: the backend chooses the machine and readies Demi's CLI
//! there, and the provider builds the spawn request from what the placement
//! found.
//!
//! Every process of an account runs in that account's one directory, the CLI's
//! configuration directory, and writes its session into a directory of its own
//! under it, which goes when the process does. A sign-in runs in a new
//! directory, which becomes the account's when it succeeds. Account work that
//! belongs to no conversation, a sign-in or a usage probe, reaches the machine
//! through an [`AccountMachine`], from any thread.
//!
//! The CLI runs on the Claude Code machine, always (one gVisor sandbox that
//! belongs to no user and in which nobody has a shell), whatever the
//! conversation's execution target is. So each account's sign-in lives in one
//! directory, and the CLI has one install to keep and one platform to be right
//! on.
//!
//! A placement is that one rule in one place: what needs a provider's process
//! says what the work is, and does not know why it got that machine. The
//! process is retained between turns and is not activity: a machine with no
//! work left goes idle and stops with its processes, and the next request wakes
//! it and resumes the session.
//!
//! A process provider cannot build its runtime from the provider alone, since
//! only the backend chooses the machine. So `Provider::runtime` refuses
//! (`ProcessHostRequired`) and the runtime is built over a placement, from the
//! arguments the provider was built from, so that no code downcasts a provider
//! to its concrete type:
//!
//! ```text
//! backend (user's shard)                      this crate
//!   claude-code family --same arguments-->    provider + placement = runtime
//!   Placement::start <------ a run needs a process
//!     wake or admit the machine, ensure the CLI, hand over an earlier sign-in
//!     spawn request built by the provider; Host process interface starts it
//! ```
//!
//! The backend holds the machine's admission only while the process starts.
//! These fail the request as one that got no answer, `overloaded`, which the
//! agent retries:
//!
//! - a failure to wake the machine, to install the CLI or to start it;
//! - a process whose machine went away.

use std::any::Any;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use demi_host_interface::{HostError, Process, SpawnRequest};
use futures_util::future::{BoxFuture, LocalBoxFuture};
use tokio_util::sync::CancellationToken;

use crate::prepared::PreparedProcesses;

/// Where a new CLI process runs on the machine the placement chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliSite {
    /// The absolute path of Demi's CLI executable there.
    pub executable: String,
    /// The process's working directory, `~/run`.
    pub run_dir: String,
    /// The CLI's configuration directory: the account's directory, where
    /// its sign-in is, or a sign-in's new directory; private to the
    /// machine's user, outside every workspace.
    pub config_dir: String,
    /// The name of the process's own directory under the configuration
    /// directory's `projects`, which holds its session and goes with it
    /// (`CLAUDE_CODE_PROJECT_DIR_NAME`).
    pub process_dir: String,
    /// What the machine tells a process there of itself.
    pub system: CliSystem,
}

/// What the CLI reads of its machine to describe its environment, which
/// the session it resumes holds so that it describes none
/// (`crates/provider-claude-code/src/session.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliSystem {
    /// The kernel's name, as `uname -s` prints it, such as `Linux`.
    pub kernel: String,
    /// The kernel's release, as `uname -r` prints it.
    pub release: String,
    /// The machine's `SHELL`; none when it names none.
    pub shell: Option<String>,
    /// The run directory as a process there sees it, its links resolved.
    pub working_directory: String,
}

/// What a start puts on the machine: the process, and the files its own
/// directory holds before it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliStart {
    pub spawn: SpawnRequest,
    /// Each file's path relative to the process's own directory, and its
    /// bytes.
    pub files: Vec<(String, Bytes)>,
}

/// What a start is for: a process of an account, in the account's
/// directory, or a sign-in, in a new one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliWork {
    Account {
        /// The account's id, which names its directory.
        account: String,
        /// The sign-in an earlier release kept in the account's record, as the
        /// CLI's own files, to write into the account's directory before the
        /// process starts unless the directory holds a sign-in already
        /// (`crates/provider-claude-code/src/account.rs`).
        handover: Option<Handover>,
    },
    Login,
}

/// A sign-in handed over to the account's directory: the CLI's files, each by
/// its name there and its bytes, written private to the machine's user.
///
/// The last is the one whose presence means the directory holds a sign-in. Each
/// file is put in place whole, in order, so the directory holds a sign-in only
/// once every file is there, and one that holds it already takes none of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handover {
    pub files: Vec<(String, Bytes)>,
}

/// Why a CLI process could not be started: the machine could not be reached,
/// Demi's CLI could not be installed there, or the Host refused the start.
///
/// The message says why, with the version when an install failed, and is the
/// request's failure as it is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct StartError(pub String);

/// What a start waits for, in its order, which a sign-in shows its user
/// (`crates/provider-claude-code/src/account.rs`). A start reports each stage as
/// it reaches it, `Install` only when the machine has no CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartStage {
    /// The machine: its access acquired, a stopped one woken, a runner
    /// that reconnects awaited.
    Machine,
    /// Demi's CLI installed on the machine.
    Install,
    /// The process readied and started.
    Process,
}

/// What says what to start at the site a placement found, given to the
/// machine's side with the start: it is built from the request where the
/// request is, and runs where the machine is.
pub type CliStarter = Arc<dyn Fn(&CliSite) -> CliStart + Send + Sync>;

/// A hold of the placement's machine for one piece of work in flight on it: a
/// run, a sign-in or a usage probe, for its whole life.
///
/// The machine does not stop while one is held, and taking one wakes it
/// (`crates/backend-cloud/src/lib.rs`). Dropping it lets go.
pub struct WorkLease(#[allow(dead_code, reason = "held for its drop")] Box<dyn Any>);

impl WorkLease {
    /// A lease that lets go when `held` is dropped.
    pub fn new(held: impl Any) -> Self {
        Self(Box::new(held))
    }
}

/// Starts the CLI processes of a runtime, a sign-in or a usage probe, on a
/// machine the placement chooses. It lives on the shard of the user whose
/// work it serves.
pub trait Placement {
    /// Starts a new CLI process for `work`.
    ///
    /// 1. The placement readies the machine, Demi's CLI, the configuration
    ///    directory (the account's, with its handover, or a sign-in's new one)
    ///    and the process's own directory under it.
    /// 2. `start` says what to start at the site it found.
    /// 3. The placement writes its files into the process's directory and
    ///    starts its request on the machine's Host.
    ///
    /// It reports each stage it reaches to `stages`. Dropping the future gives
    /// the start up, and removes what it made.
    fn start<'a>(
        &'a self,
        work: CliWork,
        start: CliStarter,
        stages: &'a dyn Fn(StartStage),
    ) -> LocalBoxFuture<'a, Result<Placed, StartError>>;

    /// A hold of the machine for a piece of work starting now, taken at
    /// once; the machine wakes meanwhile when it is stopped.
    fn work(&self) -> WorkLease;

    /// The processes prepared ahead on the placement's machine and not yet
    /// used, which every placement of that machine shares
    /// (`crates/provider-claude-code/src/prepared.rs`).
    fn prepared(&self) -> Rc<PreparedProcesses>;
}

/// A started CLI process and its own directory, with how long each step of
/// its start took.
pub struct Placed {
    pub process: Process,
    pub config: Box<dyn ConfigDir>,
    pub steps: StartSteps,
}

/// How long each step of a start took on the placement's machine, which a
/// run's span carries (`crates/provider-claude-code/src/prepared.rs`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StartSteps {
    /// Waking or admitting the machine.
    pub admit: Duration,
    /// Finding Demi's CLI there, installing it first when the machine has
    /// none.
    pub executable: Duration,
    /// Making the run directory and the process's configuration directory.
    pub directories: Duration,
    /// Reading what the machine tells a process of itself: what its
    /// runner's connection reported, and the run directory's real path on
    /// the connection's first start.
    pub system: Duration,
    /// Writing the start's files, the session among them, and how many
    /// bytes they hold.
    pub files: Duration,
    pub file_bytes: u64,
    /// Starting the process on the machine's Host.
    pub spawn: Duration,
}

/// What a process made of its own on its machine: an account's process,
/// its own directory under the account's; a sign-in, its new configuration
/// directory.
pub trait ConfigDir {
    /// The file `name` the process wrote into its configuration directory.
    fn read<'a>(&'a self, name: &'a str) -> LocalBoxFuture<'a, Result<Bytes, HostError>>;

    /// Removes what the process made once it has ended. One whose handle is
    /// dropped instead is removed the same way, by the placement, without
    /// anyone waiting for it.
    fn remove(self: Box<Self>) -> LocalBoxFuture<'static, ()>;

    /// Makes a sign-in's directory, once its process ended with success, the
    /// directory of `account`.
    ///
    /// No process of the account starts meanwhile, every one it has is closed
    /// and awaited, and its directory, when it has one, is replaced by this
    /// one. A start that waited starts in the new one
    /// (`crates/provider-claude-code/src/account.rs`). Dropping the future
    /// before the directory is replaced gives the adoption up, and the
    /// sign-in's directory goes. An account's process has nothing to adopt.
    fn adopt<'a>(self: Box<Self>, account: &'a str) -> LocalBoxFuture<'a, Result<(), HostError>>;
}

/// The work a sign-in or a usage probe does on its machine: given the
/// placement of that machine and a token that fires when nobody waits for
/// the work any more, it runs to its end.
pub type AccountWork =
    Box<dyn FnOnce(Rc<dyn Placement>, CancellationToken) -> LocalBoxFuture<'static, ()> + Send>;

/// The machine account work runs on, the Claude Code machine,
/// as the backend gives it to a sign-in
/// and a usage probe from any thread.
pub trait AccountMachine: Send + Sync {
    /// Runs `work` for the provider entry `entry`, or the login that is to make
    /// it, where the machine is reached, and answers once it ended.
    ///
    /// Dropping the answer fires the token `work` received; `work` runs to its
    /// end either way.
    fn run(&self, entry: String, work: AccountWork) -> BoxFuture<'static, Result<(), StartError>>;

    /// Deletes the directory of the removed account `account`.
    ///
    /// No process of it starts meanwhile, every one it has is closed and
    /// awaited first, and a start that waited fails with the removal
    /// (`crates/provider-claude-code/src/account.rs`).
    fn remove_account(&self, account: String) -> BoxFuture<'static, Result<(), StartError>>;
}
