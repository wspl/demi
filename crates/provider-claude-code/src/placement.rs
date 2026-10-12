//! Where the CLI runs (`claude-code.md` § How a runtime gets its process, §
//! Accounts and sign-in): the backend chooses the machine and readies Demi's
//! CLI there, and the provider builds the spawn request from what the
//! placement found. Every process gets a configuration directory of its
//! own, private to the machine's user, which goes when the process does.
//! Account work that belongs to no conversation, a sign-in, reaches its
//! machine through an [`AccountMachine`], from any thread.

use std::rc::Rc;
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
    /// The process's working directory, `~/.demi/claude/run`.
    pub run_dir: String,
    /// The process's own configuration directory: new, private to the
    /// machine's user, outside every workspace, holding only the files the
    /// start put there.
    pub config_dir: String,
    /// What the machine tells a process there of itself.
    pub system: CliSystem,
}

/// What the CLI reads of its machine to describe its environment, which
/// the session it resumes holds so that it describes none
/// (`claude-code.md` § The session a process resumes).
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

/// What a start puts on the machine: the process, and the files its
/// configuration directory holds before it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliStart {
    pub spawn: SpawnRequest,
    /// Each file's path relative to the configuration directory, and its
    /// bytes.
    pub files: Vec<(String, Bytes)>,
}

/// Why a CLI process could not be started: the machine could not be reached,
/// Demi's CLI could not be installed there, or the Host refused the start.
/// The message says why, with the version when an install failed, and is
/// the request's failure as it is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct StartError(pub String);

/// Starts the CLI processes of a runtime or a sign-in, on a machine the
/// placement chooses. It lives on the user's shard with its user.
pub trait Placement {
    /// Starts a new CLI process: the placement readies the machine, Demi's
    /// CLI and a new configuration directory there, `start` says what to
    /// start at the site it found, and the placement writes its files into
    /// the directory and starts its request on the machine's Host. Dropping
    /// the future gives the start up, and removes the directory.
    fn start<'a>(
        &'a self,
        start: &'a dyn Fn(&CliSite) -> CliStart,
    ) -> LocalBoxFuture<'a, Result<Placed, StartError>>;

    /// The processes prepared ahead on the placement's machine and not yet
    /// used, which every placement of that machine shares
    /// (`claude-code.md` § Preparing a process ahead).
    fn prepared(&self) -> Rc<PreparedProcesses>;
}

/// A started CLI process and its configuration directory, with how long
/// each step of its start took.
pub struct Placed {
    pub process: Process,
    pub config: Box<dyn ConfigDir>,
    pub steps: StartSteps,
}

/// How long each step of a start took on the placement's machine, which a
/// run's span carries (`claude-code.md` § Preparing a process ahead).
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

/// A process's configuration directory on its machine.
pub trait ConfigDir {
    /// The file `name` the process wrote there.
    fn read<'a>(&'a self, name: &'a str) -> LocalBoxFuture<'a, Result<Bytes, HostError>>;

    /// Removes the directory once its process has ended. A directory whose
    /// handle is dropped instead is removed the same way, by the placement,
    /// without anyone waiting for it.
    fn remove(self: Box<Self>) -> LocalBoxFuture<'static, ()>;
}

/// The work a sign-in does on its machine: given the placement of that
/// machine and a token that fires when nobody waits for the work any more,
/// it runs to its end.
pub type AccountWork =
    Box<dyn FnOnce(Rc<dyn Placement>, CancellationToken) -> LocalBoxFuture<'static, ()> + Send>;

/// The machine account work runs on, the acting user's Cloud
/// (`claude-code.md` § Where it runs), as the backend gives it to a sign-in
/// from any thread.
pub trait AccountMachine: Send + Sync {
    /// Runs `work` for the provider entry `entry`, or the login that is to
    /// make it, where the machine is reached, and answers once it ended.
    /// Dropping the answer fires the token `work` received; `work` runs to
    /// its end either way.
    fn run(&self, entry: String, work: AccountWork) -> BoxFuture<'static, Result<(), StartError>>;
}
