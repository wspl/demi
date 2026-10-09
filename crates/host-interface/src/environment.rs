//! The shell-environment contract behind the shell tools: a node's shells
//! on one Host, the commands they run, the model's status view of each
//! (`runtime.md` § Tools), and the pages' view of each, which the
//! environment reports to the node's feed (`runtime.md` § Live output).

use std::{cell::RefCell, rc::Rc, sync::Arc, time::Duration};

use bytes::Bytes;
use demi_shared_types::{
    BinaryStdout, CommandId, EditedFile, NodeId, OutputView, Sequence, ShellId, StreamKind,
    StreamView,
};
use futures_util::future::LocalBoxFuture;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::{CommandMedium, CommandRecord, Ending, HostError, PageView, Seen, WholeOutput};

/// The longest a call watches its command before it returns the running
/// command's handle.
pub const MAX_OBSERVATION: Duration = Duration::from_millis(600_000);

/// The default budget of one status view's new output, per stream.
pub const DEFAULT_OUTPUT_LIMIT_BYTES: usize = 1024 * 1024;

/// The most of a binary final stdout a status carries: enough for an
/// ordinary clip to reach the tools that decide what to do with it, and a
/// bound on a runaway producer.
pub const DEFAULT_BINARY_LIMIT_BYTES: usize = 16 * 1024 * 1024;

/// A node's shells on one Host. Its handles belong to it: a command id or
/// shell id another environment made is unknown here. Every status it
/// answers is the model's; the pages see its commands through the feed it
/// was made with ([`PageFeed`]).
pub trait ShellEnvironment {
    /// Starts `request.script` and returns its command's handle; the command
    /// goes on by itself, and `cancel` stops it whenever it is cancelled.
    /// [`watch`] watches it.
    fn start(
        &self,
        request: ExecRequest,
        cancel: CancellationToken,
    ) -> LocalBoxFuture<'_, Result<CommandId, ShellError>>;

    /// Waits until the command has ended, and answers how: at once for one
    /// that has ended already.
    fn ended<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, Result<Ending, ShellError>>;

    /// The command's status, with its output since the model last looked.
    fn status(&self, command: &CommandId) -> Result<CommandStatus, ShellError>;

    /// The default shell and its directory now, where a new shell beside it
    /// starts; none while the environment has no default shell, whose first
    /// command starts in the Host's default directory.
    fn default_shell(&self) -> Option<DefaultShell>;

    /// The kept output of a command that runs, as its Host holds it now
    /// (`runtime.md` § The whole output).
    fn read_output<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<WholeOutput, ShellError>>;

    /// Medium `number` of a command that runs, as its Host keeps it
    /// (`runtime.md` § The whole output).
    fn read_medium<'a>(
        &'a self,
        command: &'a CommandId,
        number: u32,
    ) -> LocalBoxFuture<'a, Result<Bytes, ShellError>>;

    /// Writes to a running command's standard input.
    fn write<'a>(
        &'a self,
        command: &'a CommandId,
        stdin: Bytes,
    ) -> LocalBoxFuture<'a, Result<(), ShellError>>;

    /// Stops a running command: asks it to end, and ends it when it does not.
    /// A command that is not running stays as it is.
    fn abort<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, Result<(), ShellError>>;

    /// The pages' view of every command the environment holds: each one
    /// that runs, and each one that ended and is not released.
    fn page_views(&self) -> Vec<PageView>;

    /// Forgets a command, stopping it first when it runs; false when unknown.
    fn release_command<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, bool>;

    /// Ends a shell, stopping its command; false when unknown.
    fn dispose_shell<'a>(&'a self, shell: &'a ShellId) -> LocalBoxFuture<'a, bool>;

    /// Ends every shell.
    fn dispose_all(&self) -> LocalBoxFuture<'_, ()>;

    fn owns_shell(&self, shell: &ShellId) -> bool;

    fn owns_command(&self, command: &CommandId) -> bool;
}

/// Where an environment's command and shell numbers come from: the
/// conversation's sequences, which give each number once, in order
/// (`runtime.md` § Identifiers the model sees).
pub trait Numbers {
    /// The conversation's next number of `sequence`.
    fn next(&self, sequence: Sequence) -> LocalBoxFuture<'_, Result<u64, HostError>>;
}

/// Where a node's shell environments tell the pages of their commands
/// (`runtime.md` § Live output), and learn whether a page watches.
pub trait PageFeed {
    /// The pages' view of the command `record` holds changed: the command
    /// started, its output grew, or it ended. An environment reports nothing
    /// of a command after its end.
    fn changed(&self, record: &Rc<RefCell<CommandRecord>>);

    /// Whether a page watches now, and each change of it: while one does,
    /// the environments' jobs send their output beyond what the model's view
    /// holds.
    fn watching(&self) -> watch::Receiver<bool>;
}

/// Watches `command` until the first of: it ends, `window` passes, or
/// `until` resolves; then returns its status, with what `until` gave when
/// it ended the watch (`runtime.md` § The window). Without a window it
/// looks at once. Ending the watch never stops the command.
pub async fn watch<T>(
    environment: &dyn ShellEnvironment,
    command: &CommandId,
    window: Option<ObservationWindow>,
    until: impl Future<Output = T>,
) -> Result<(CommandStatus, Option<T>), ShellError> {
    let mut ended_by = None;
    if let Some(window) = window {
        tokio::select! {
            ended = environment.ended(command) => {
                ended?;
            }
            () = tokio::time::sleep(window.duration()) => {}
            reason = until => ended_by = Some(reason),
        }
    }
    Ok((environment.status(command)?, ended_by))
}

/// An environment's default shell, and the directory it is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultShell {
    pub id: ShellId,
    pub cwd: String,
}

/// One exec, with every rule an environment enforces in its type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecRequest {
    pub script: String,
    pub shell: ShellTarget,
    /// The agent node the job's `rpc` calls act for.
    pub caller: JobCaller,
    /// The `shell_exec` call that runs the script, which the pages' view of
    /// the command names.
    pub tool_use_id: String,
}

/// Which shell an exec runs in (`runtime.md` § Dispatch and failures).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellTarget {
    /// The environment's default shell, or, while that one runs a command,
    /// a new shell that starts in its directory, so a long command never
    /// blocks the next exec.
    Default,
    /// This shell; it must be idle.
    Existing(ShellId),
    /// A new shell that starts in `cwd`, the Host's default when none, so
    /// its directory changes reach no other shell.
    New { cwd: Option<String> },
}

/// How long a call watches its command: from a millisecond to
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


/// The agent node a job runs for, whose commands its `rpc` calls reach
/// (`sessions-and-targets.md` § Bind jobs to their caller).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobCaller {
    pub node: NodeId,
}

/// A command's status and its output since the last look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandStatus {
    pub shell_id: ShellId,
    pub command_id: CommandId,
    /// While the command runs: each stream's start, which the backend holds.
    pub stdout: StreamView,
    pub stderr: StreamView,
    pub output: OutputView,
    /// While the command runs, the bytes its Host holds beyond what the
    /// views hold (`runtime.md` § Results and previews).
    pub unreceived: u64,
    /// While the command runs, the newest bytes of each stream that grew
    /// beyond its start, as the runner last sent them.
    pub newest: Vec<Newest>,
    /// Once the command ended, its whole output.
    pub whole: Option<WholeView>,
    pub running_ms: u64,
    pub idle_ms: u64,
    pub state: CommandState,
    /// The files the command changed, once it ended having changed some.
    pub files: Option<EditedFiles>,
}

/// A stream's newest bytes beyond its start, which the runner sends while
/// a command runs (`runner.md` § Pipes and output).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Newest {
    pub stream: StreamKind,
    /// Where they start in the stream, in bytes.
    pub offset: u64,
    /// The bytes between the stream's start the views hold and these.
    pub left_out: u64,
    /// Their text; a character cut at their start is left out.
    pub text: String,
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
        /// The media its declared commands returned to the job, by number.
        media: Vec<CommandMedium>,
    },
    /// It was stopped.
    Aborted,
}

/// The whole output of a command that ended, and how many bytes of each
/// stream the model had seen before this look; everything when a look gave
/// the whole output already.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WholeView {
    pub output: Arc<WholeOutput>,
    pub seen: Seen,
}

/// A final stdout that was not text: its bytes when the output kept all of
/// them within the binary limit, and their description.
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
    #[error("command {command} has no medium {number}: it returned {returned}")]
    NoMedium {
        command: CommandId,
        number: u32,
        returned: usize,
    },
    #[error("stdin must not be empty")]
    EmptyStdin,
    #[error(transparent)]
    Host(#[from] HostError),
}
