//! The job shell contract (`runner.md` § Shell jobs): how the runner starts
//! a job's script, feeds its input, signals, cancels and awaits it, without
//! knowing which shell runs it.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::PathBuf,
    sync::Arc,
};

use demi_command_protocol::LocalInvocation;
use demi_command_sdk::{Handler, edits::Recorder};
use demi_runner_protocol::wire::Signal;
use futures_util::future::BoxFuture;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::process::{OutputChunk, ProcessExit, ProcessInput};

/// A job's declared commands, which its shell runs as builtins, and the
/// handler that takes each invocation of one.
#[derive(Clone)]
pub struct JobCommands {
    /// The job's execution context, which every invocation names and the
    /// programs the job starts find in `DEMI_CONTEXT_ID`.
    pub context: String,
    /// The root commands the job's manifest declares.
    pub roots: Vec<String>,
    pub handler: Arc<dyn Handler<Metadata = LocalInvocation>>,
}

/// What a job starts with.
pub struct JobStart {
    pub script: String,
    /// Where the job starts; a directory that does not exist fails the job
    /// before its script runs (`runner.md` § Shell jobs).
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    /// Whether the job's input is its own input rather than a stdin the
    /// backend relays (`runner.md` § A job's own input).
    pub live: bool,
    /// Ends the job and everything it runs when cancelled.
    pub cancellation: CancellationToken,
    pub commands: Option<JobCommands>,
    /// Records the files the job changes (`edit-tracking.md`).
    pub edits: Option<Recorder>,
}

/// The shell that runs the runner's jobs.
pub trait JobShell: Send + Sync {
    fn start(&self, job: JobStart) -> BoxFuture<'_, io::Result<Box<dyn ShellJob>>>;

    /// The names the shell's own builtins take, which no declared root
    /// command may take.
    fn builtin_names(&self) -> BTreeSet<String>;
}

/// One running job. Dropping it cancels the job.
pub trait ShellJob: Send + Sync {
    /// Where the job's input goes.
    fn input(&self) -> &mpsc::Sender<ProcessInput>;

    /// The job's output, chunk by chunk, until everything it ran has
    /// finished.
    fn output(&mut self) -> &mut mpsc::Receiver<OutputChunk>;

    /// Stops the job for a signal that ends it, which its exit reports:
    /// `KILL` at once, the others by ending its shell work and signalling
    /// every process group it started (`runner.md` § Cancellation and
    /// completion); other signals are refused.
    fn signal(&self, signal: Signal) -> io::Result<()>;

    fn cancel(&self);

    fn is_cancelled(&self) -> bool;

    /// The job's exit, once everything it ran has finished.
    fn wait(&mut self) -> BoxFuture<'_, ProcessExit>;
}
