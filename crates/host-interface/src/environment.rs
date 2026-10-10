//! The shell-environment contract behind the `shell` tool: a node's commands
//! on one Host, each a job of its own that starts in the conversation's
//! working directory there (`runtime.md` § Running shell tools), the
//! model's status view of each (`runtime.md` § Tools), and the pages' view
//! of each, which the environment reports to the node's feed (`runtime.md`
//! § Live output).

use std::{cell::RefCell, rc::Rc, sync::Arc, time::Duration};

use bytes::Bytes;
use demi_shared_types::{
    BinaryStdout, CommandId, EditedFile, NodeId, OutputView, PathChange, Sequence, StreamKind,
    StreamView, Unreachable,
};
use demi_command_protocol::Viewable;
use futures_util::future::LocalBoxFuture;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::{CommandMedium, CommandRecord, Ending, HostError, PageView, Seen, WholeOutput};

/// The default budget of one status view's new output, per stream.
pub const DEFAULT_OUTPUT_LIMIT_BYTES: usize = 1024 * 1024;

/// The most of a binary final stdout a status carries: enough for an
/// ordinary clip to reach the tools that decide what to do with it, and a
/// bound on a runaway producer.
pub const DEFAULT_BINARY_LIMIT_BYTES: usize = 16 * 1024 * 1024;

/// A node's commands on one Host. Its handles belong to it: a command id
/// another environment made is unknown here. Every status it answers is the
/// model's; the pages see its commands through the feed it was made with
/// ([`PageFeed`]).
pub trait ShellEnvironment {
    /// Starts `request.script` as a job of its own in the conversation's
    /// working directory on the Host, carrying nothing of an earlier
    /// command, and returns its command's handle; the command goes on by
    /// itself, and `cancel` stops it whenever it is cancelled. [`watch`]
    /// watches it.
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

    /// The media the command's job viewed, once it exited, which moves no
    /// place in its output (`runtime.md` § What a result attaches).
    fn media(&self, command: &CommandId) -> Result<Vec<CommandMedium>, ShellError>;

    /// How long the command has printed nothing, which moves no place in
    /// its output.
    fn quiet(&self, command: &CommandId) -> Result<Duration, ShellError>;

    /// The command lines of the background tasks that keep `command`
    /// running once its script has ended, which moves no place in its
    /// output (`runtime.md` § Results and previews); empty while its script
    /// runs, and for a command that ended or that the environment does not
    /// hold.
    fn outliving(&self, command: &CommandId) -> Vec<String>;

    /// How long the Host of `command`, which runs, has been unreachable as
    /// far as the environment knows, and how long its runner keeps it so
    /// (`runtime.md` § Command reports); none while its connection serves
    /// it, and for a command that ended or that the environment does not
    /// hold.
    fn unreachable(&self, command: &CommandId) -> Option<Unreachable>;

    /// The kept output of a command that runs, as its Host holds it now
    /// (`runtime.md` § The whole output).
    fn read_output<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<WholeOutput, ShellError>>;

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

    /// Takes up `taken`, a command that still ran when the backend last
    /// knew it (`sessions-and-targets.md` § Recovery and persistence).
    fn adopt(&self, taken: TakenUp);

    /// Lets go of every command, which runs on, as a disposed node does
    /// (`runtime.md` § Dispose and restore); waits until the environment
    /// follows none.
    fn detach_all(&self) -> LocalBoxFuture<'_, ()>;

    /// Stops every command that runs, and waits until each has ended.
    fn dispose_all(&self) -> LocalBoxFuture<'_, ()>;

    fn owns_command(&self, command: &CommandId) -> bool;
}

/// Where an environment's command numbers come from: the
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

/// One exec, with every rule an environment enforces in its type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecRequest {
    pub script: String,
    /// The agent node the job's `rpc` calls act for.
    pub caller: JobCaller,
    /// The `shell` call that runs the script, which the pages' view of the
    /// command names.
    pub tool_use_id: String,
    /// The media types the call's model reads in a tool result, which the
    /// job may show it (`runtime.md` § What `demi file view` shows).
    pub viewable: Viewable,
}

/// The agent node a job runs for, whose commands its `rpc` calls reach
/// (`sessions-and-targets.md` § Bind jobs to their caller).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobCaller {
    pub node: NodeId,
}

/// A command an environment takes up again: the call `tool_use_id`
/// started it as the job `job` on the environment's Host for the node
/// `caller`, it has run for `running` since, and the node had seen `seen`
/// of its output (`storage.md` § Command outputs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakenUp {
    pub command: CommandId,
    pub tool_use_id: String,
    pub job: String,
    pub caller: JobCaller,
    pub running: Duration,
    pub seen: Seen,
}

/// A command's status and its output since the last look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandStatus {
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
    /// model, when it declares one, and `outliving` the command lines of the
    /// background tasks that keep it running once its script has ended,
    /// empty while the script runs.
    Running {
        hint: Option<String>,
        outliving: Vec<String>,
    },
    Exited {
        exit_code: i32,
        /// Present when the final stdout was not text.
        binary_stdout: Option<BinaryOutput>,
        /// The media its declared commands returned to the job, by number.
        media: Vec<CommandMedium>,
    },
    /// It was stopped.
    Aborted,
    /// Its Host lost it, for `reason` (`runtime.md` § Lost commands), with
    /// the media its declared commands returned to the job, which went with
    /// it.
    Lost {
        reason: String,
        media: Vec<CommandMedium>,
    },
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
    /// The renames and removals of its embedded utilities, in order.
    pub path_changes: Vec<PathChange>,
    /// Whether the list was cut.
    pub truncated: bool,
}

/// Why an environment refused a request. The texts are the model's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShellError {
    #[error("Unknown command \"{0}\"")]
    UnknownCommand(CommandId),
    #[error("Command \"{0}\" is not running")]
    NotRunning(CommandId),
    #[error("stdin must not be empty")]
    EmptyStdin,
    #[error(transparent)]
    Host(#[from] HostError),
}
