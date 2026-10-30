//! The embedded brush shell and the standard utilities, which run in each
//! job's shell process (`remote`).
//!
//! The crate holds:
//!
//! - a fresh shell per job;
//! - the job's shell process, and the runner's end of it (`remote`);
//! - the builtins that act on a process;
//! - the declared commands' builtins, which hand each invocation to the handler
//!   the job supplies;
//! - the job shell contract's implementation.
//!
//! A shell job owns its working directory, environment, IO and asynchronous
//! work, and carries the command context the backend built for it (the runner
//! never derives that from the job's environment). Brush runs in the job's
//! shell process, on a runtime of its own. Every in-process file write passes
//! through the job's scope, which reports what the job created or modified
//! when it exits (edit tracking).
//!
//! Each job has a process of its own, which outlives the runner, so a runner
//! that updates or crashes leaves it running, and one shell that fails ends
//! its own job alone:
//!
//! ```text
//! Runner process          Job A's shell process   Job B's shell process
//! +----------------+      +--------------------+  +--------------------+
//! | Job table      |<---->| Brush execution    |  | Brush execution    |
//! | RemoteShell    |<--+  | Background tasks   |  | Background tasks   |
//! +----------------+   |  | IO and child work  |  | IO and child work  |
//!                      |  +--------------------+  +--------------------+
//!                      +-------------------------------------^
//! ```
//!
//! # Environment
//!
//! - The runner sets no `TMPDIR` for a job: it is the device environment's. A
//!   temporary directory of the job's own would break dev servers. macOS limits
//!   a Unix socket's path to 104 bytes, which a path under the job root
//!   exceeds, and a daemon the job started would outlive the directory.
//! - A job's `HOME` is the request's, else the device environment's, else the
//!   home the runner reports. A runner a service manager starts without `HOME`
//!   still has its jobs read `~/.profile`, where brush with `$HOME` unset would
//!   look for `/.local/bin/env`.
//!
//! # Utilities
//!
//! A utility that writes files or starts programs runs in the job's shell, so that
//! its changes are the job's edits, and so do `sh` and `bash`, which are the
//! job's own shell on every Host (`nested.rs`) so that what they run is
//! recorded too. Every other utility is the system's own: GNU's on Linux,
//! macOS's on a Mac, Git for Windows' on Windows (`utilities.rs`).
//!
//! On Windows the shell and its utilities read a drive written as Git Bash
//! writes it (`/c/Users/…`) as `C:\Users\…`, and `/tmp` as the temporary
//! folder, as Git's utilities do; a program started by name takes only the
//! Windows form (`C:/Users/…`). A script file runs by the interpreter its
//! first line names, as Git Bash does (`shebang.rs`).
//!
//! # Boundaries
//!
//! The crate knows no jobs, execution contexts, residents or connection: a
//! declared command reaches the dispatcher only through the `command-sdk`
//! `Handler` the job gives it. It is the only crate that links brush and the
//! standard utilities, and nothing below the executable depends on it, so a
//! change elsewhere recompiles none of them.

mod declared;
mod git_paths;
mod interpreter;
mod job;
mod nested;
mod nohup;
mod process_builtins;
pub mod remote;
mod scope;
mod shebang;
mod signals;
mod timeout;
mod timing;
mod utilities;
mod which;
mod work;

use demi_runner_process::job_shell::{JobShell, JobStart, ShellJob};
use futures_util::future::BoxFuture;
use std::{collections::BTreeSet, io};

/// The runner's own environment as a shell of this platform reads it: on
/// Windows the well-known names in their POSIX spelling, such as `Path` as
/// `PATH`, which brush does for the environment it inherits.
///
/// A job's shell does not inherit the runner's environment but is given one
/// built from this. Without it a job on Windows would find `PATH` holding only
/// the alias directory and run no program of the device.
pub fn host_environment() -> impl Iterator<Item = (String, String)> {
    brush_core::sys::env::get_host_env_vars()
}

/// The shell runtime (`crates/runner/src/main.rs`): every interpreter unit
/// and utility of a job runs on a thread of its blocking pool, apart from the
/// control thread. A unit blocked on a full pipe waits for a sibling that has
/// a thread of its own, so the pool takes a thread for every unit and has no
/// bound of Demi's: the system's own limit on threads is the only one.
#[derive(Clone)]
pub struct ShellRuntime(tokio::runtime::Handle);

impl ShellRuntime {
    /// One worker drives the units' asynchronous parts, such as reaping
    /// children; the blocking pool runs the units themselves.
    pub fn build() -> io::Result<tokio::runtime::Runtime> {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            // Tokio takes a bound; this is as large as it takes without
            // overflowing its own sums.
            .max_blocking_threads(usize::MAX / 2)
            .thread_name("shell")
            .enable_all()
            .build()
    }

    pub fn new(runtime: &tokio::runtime::Runtime) -> Self {
        Self(runtime.handle().clone())
    }

    /// The runtime the caller runs on, for tests that host jobs themselves.
    pub fn current() -> Self {
        Self(tokio::runtime::Handle::current())
    }

    /// Runs `work` on a thread of the shell pool, inside the shell runtime,
    /// so what it spawns stays there too.
    pub fn spawn_blocking<F, R>(&self, work: F) -> tokio::task::JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        self.0.spawn_blocking(work)
    }
}

/// Runs each job in a fresh shell of its own.
impl JobShell for ShellRuntime {
    fn start(&self, job: JobStart) -> BoxFuture<'_, io::Result<Box<dyn ShellJob>>> {
        Box::pin(async move {
            let mut scope = scope::Scope::new(job.cancellation, job.commands);
            scope.work.hold(job.lease);
            scope.edits = job.edits;
            let job = job::Job::start(
                job.script,
                job.cwd,
                job.env,
                job.live,
                scope,
                self,
            )
            .await?;
            Ok(Box::new(job) as Box<dyn ShellJob>)
        })
    }

    fn builtin_names(&self) -> BTreeSet<String> {
        interpreter::registrations().into_keys().collect()
    }
}
