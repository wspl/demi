//! The shell-environment contract behind the `shell_*` tools: a node's shells
//! on one Host, the commands they run, and the model's status view of each
//! (`runtime.md` § Tools).

use std::time::Duration;

use bytes::Bytes;
use demi_core::{BinaryStdout, CommandId, EditedFile, NodeId, OutputView, ShellId, StreamView};
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

use crate::HostError;

/// The longest an exec waits for its command before it returns the running
/// command's handle.
pub const MAX_OBSERVATION: Duration = Duration::from_millis(600_000);

/// How long an exec waits when its caller names no window.
pub const DEFAULT_OBSERVATION: Duration = Duration::from_millis(10_000);

/// The default budget of one status view's new output, per stream.
pub const DEFAULT_OUTPUT_LIMIT_BYTES: usize = 1024 * 1024;

/// The most of a binary final stdout a status carries: enough for an
/// ordinary clip to reach the tools that decide what to do with it, and a
/// bound on a runaway producer.
pub const DEFAULT_BINARY_LIMIT_BYTES: usize = 16 * 1024 * 1024;

/// A node's shells on one Host. Its handles belong to it: a command id or
/// shell id another environment made is unknown here.
pub trait ShellEnvironment {
    /// Runs `request.script`, and returns once the command ended or its
    /// observation window passed, whichever is first; a command still
    /// running then goes on, and `cancel` stops it whenever it is cancelled.
    fn exec(
        &self,
        request: ExecRequest,
        cancel: CancellationToken,
    ) -> LocalBoxFuture<'_, Result<CommandStatus, ShellError>>;

    /// The command's status, with its output since the last look.
    fn status(&self, command: &CommandId) -> Result<CommandStatus, ShellError>;

    /// Writes to a running command's standard input.
    fn write<'a>(
        &'a self,
        command: &'a CommandId,
        stdin: Bytes,
    ) -> LocalBoxFuture<'a, Result<CommandStatus, ShellError>>;

    /// Stops a running command: asks it to end, and ends it when it does not.
    /// A command that is not running answers its status.
    fn abort<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<CommandStatus, ShellError>>;

    /// Forgets a command, stopping it first when it runs; false when unknown.
    fn release_command<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, bool>;

    /// Ends a shell, stopping its command; false when unknown.
    fn dispose_shell<'a>(&'a self, shell: &'a ShellId) -> LocalBoxFuture<'a, bool>;

    /// Ends every shell.
    fn dispose_all(&self) -> LocalBoxFuture<'_, ()>;

    fn owns_shell(&self, shell: &ShellId) -> bool;

    fn owns_command(&self, command: &CommandId) -> bool;
}

/// One exec, with every rule an environment enforces in its type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecRequest {
    pub script: String,
    pub shell: ShellTarget,
    pub window: ObservationWindow,
    /// Whose command storage the job's `rpc` calls reach.
    pub caller: JobCaller,
}

/// Which shell an exec runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellTarget {
    /// The environment's default shell, or a fresh shell while that one runs
    /// a command, so a long command never blocks the next exec.
    Default,
    /// This shell; it must be idle.
    Existing(ShellId),
    /// A shell of its own that starts in `cwd`, the Host's default when none,
    /// so directory changes leak into no other exec. Its caller disposes of
    /// it.
    Ephemeral { cwd: Option<String> },
}

/// How long an exec waits for its command: from a millisecond to
/// [`MAX_OBSERVATION`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservationWindow(Duration);

impl ObservationWindow {
    /// The window of `milliseconds`, or none outside the bounds.
    pub fn from_millis(milliseconds: u64) -> Option<Self> {
        let window = Duration::from_millis(milliseconds);
        (milliseconds > 0 && window <= MAX_OBSERVATION).then_some(Self(window))
    }

    pub fn duration(self) -> Duration {
        self.0
    }
}

impl Default for ObservationWindow {
    fn default() -> Self {
        Self(DEFAULT_OBSERVATION)
    }
}

/// The command storage a job's `rpc` calls reach: the calling node and the
/// history generation it was at when the job started
/// (`command-state-history.md` § Mutation API and concurrency).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JobCaller {
    pub node: NodeId,
    pub generation: u64,
}

/// A command's status and its output since the last look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandStatus {
    pub shell_id: ShellId,
    pub command_id: CommandId,
    /// The directory on the Host that holds the command's output files;
    /// none when nothing beyond the view is kept.
    pub output_dir: Option<String>,
    pub stdout: StreamView,
    pub stderr: StreamView,
    pub output: OutputView,
    pub running_ms: u64,
    pub idle_ms: u64,
    pub state: CommandState,
    /// The files the command changed, once it ended having changed some.
    pub files: Option<EditedFiles>,
}

/// Where a command is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandState {
    /// It runs; `hint` is the running declared command's guidance for the
    /// model, when it declares one.
    Running { hint: Option<String> },
    Exited {
        exit_code: i32,
        /// Present when the final stdout was not text.
        binary_stdout: Option<BinaryOutput>,
    },
    /// It was stopped.
    Aborted,
}

/// A final stdout that was not text: its bytes, within the binary limit, and
/// their description, which is what leaves the process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryOutput {
    pub bytes: Bytes,
    pub info: BinaryStdout,
}

/// The files a command changed, for the user; never shown to the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditedFiles {
    pub files: Vec<EditedFile>,
    /// Whether the list was cut.
    pub truncated: bool,
}

/// Why an environment refused a request. The texts are the model's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShellError {
    #[error("Unknown shell session \"{0}\"")]
    UnknownShell(ShellId),
    #[error("Unknown command \"{0}\"")]
    UnknownCommand(CommandId),
    #[error("Shell session \"{shell}\" is already running command \"{command}\"")]
    ShellBusy { shell: ShellId, command: CommandId },
    #[error("Command \"{0}\" is not running")]
    NotRunning(CommandId),
    #[error("shell_write field \"stdin\" must not be empty; use shell_status to poll")]
    EmptyStdin,
    #[error("Command \"{0}\" is still acquiring its Host")]
    Starting(CommandId),
    #[error(transparent)]
    Host(#[from] HostError),
}
