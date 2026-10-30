//! The job shell contract (`crates/runner-shell/src/lib.rs`): how the runner starts
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
use demi_runner_protocol::wire::{OutputStream, Signal};
use futures_util::future::BoxFuture;
use tokio::sync::{mpsc, oneshot, watch};
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
    /// before its script runs (`crates/runner-shell/src/lib.rs`).
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    /// Whether the job's input is its own input rather than a stdin the
    /// backend relays (`crates/runner-process/src/stdio.rs`).
    pub live: bool,
    /// Ends the job and everything it runs when cancelled.
    pub cancellation: CancellationToken,
    pub commands: Option<JobCommands>,
    /// Records the files the job changes (`crates/command-sdk/src/edits.rs`).
    pub edits: Option<Recorder>,
    /// Held until the last of what the job started is gone, which may be long
    /// after the job's end: the job's command context, which what it left
    /// running still uses (`crates/runner-shell/src/work.rs`).
    pub lease: Option<Box<dyn Send + Sync>>,
}

/// The shell that runs the runner's jobs.
pub trait JobShell: Send + Sync {
    fn start(&self, job: JobStart) -> BoxFuture<'_, io::Result<Box<dyn ShellJob>>>;

    /// The names the shell's own builtins take, which no declared root
    /// command may take.
    fn builtin_names(&self) -> BTreeSet<String>;
    /// Job `job`, which a runner before this one started, when it still runs
    /// and this runner can take it over with `lease` held until what it left
    /// running is gone; none when the shell does not outlive its runner.
    fn reattach(
        &self,
        _job: &str,
        _lease: Option<Box<dyn Send + Sync>>,
    ) -> BoxFuture<'_, io::Result<Option<Box<dyn ShellJob>>>> {
        Box::pin(async { Ok(None) })
    }
    /// Takes over the shells a runner before this one left that run on for
    /// jobs that have ended, for what they left running: their log lines,
    /// and a `kill` of their background tasks from another job. Called once
    /// the jobs that run are taken over.
    fn reattach_left(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    /// The runner leaves its jobs to the runner that starts next: from now
    /// on neither a job's cancellation nor its drop stops it.
    fn leave(&self) {}
    /// The backend has job `job`'s end, and released its directory: a shell
    /// that held the end for a runner that starts next lets it go.
    fn released(&self, _job: &str) {}
    /// The runner stops, drains or is removed: every shell it reaches ends,
    /// with the work of an ended job that still runs in it, as that work
    /// ended with a runner that ran it in its own process.
    fn close(&self) {}
}

/// One running job. Dropping it cancels the job, unless it has ended by itself:
/// what it left running then runs on.
pub trait ShellJob: Send + Sync {
    /// Where the job's input goes.
    fn input(&self) -> &mpsc::Sender<ProcessInput>;

    /// The job's output, chunk by chunk, until everything it ran has
    /// finished.
    fn output(&mut self) -> &mut mpsc::Receiver<OutputChunk>;

    /// Reads what the job's stdout pipe holds now, without waiting for
    /// more, into [`ShellJob::output`]; the answer comes once those chunks
    /// are there, so what follows them follows everything written before
    /// (`crates/plugin-file/command/src/files.rs`).
    fn drain_stdout(&self) -> oneshot::Receiver<()>;

    /// Stops the job for a signal that ends it, which its exit reports:
    /// `KILL` at once, the others by ending its shell work and signalling
    /// every process group it started (`crates/runner-shell/src/job.rs`); other signals are refused.
    fn signal(&self, signal: Signal) -> io::Result<()>;

    fn cancel(&self);

    fn is_cancelled(&self) -> bool;

    /// The background tasks that keep the job running once its script has
    /// ended, each by its command line, and each change of them as they end;
    /// empty while the script runs (`crates/agent-tools/src/result.rs`).
    fn outliving(&self) -> watch::Receiver<Vec<String>>;

    /// The job's exit, once everything it ran has finished.
    fn wait(&mut self) -> BoxFuture<'_, ProcessExit>;
    /// The runner has kept `bytes` more of the job's `stream` from
    /// [`ShellJob::output`] in the job's kept output. A shell in a process of
    /// its own holds what the runner has not kept, so that a runner that
    /// starts again reads none twice and misses none
    /// (`crates/runner-shell/src/remote/mod.rs`).
    fn kept(&self, _stream: OutputStream, _bytes: u64) {}
}
