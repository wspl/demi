//! The embedded brush shell and the standard utilities, which run in the
//! runner's process.
//!
//! The crate holds:
//!
//! - a fresh shell per job;
//! - the builtins that act on a process;
//! - the declared commands' builtins, which hand each invocation to the handler
//!   the job supplies;
//! - the job shell contract's implementation.
//!
//! A shell job owns its working directory, environment, IO and asynchronous
//! work, and carries the command context the backend built for it (the runner
//! never derives that from the job's environment). Brush runs inside the runner
//! on a runtime of its own, apart from the control thread. Every in-process
//! file write passes through the job's scope, which reports what the job
//! created or modified when it exits (edit tracking).
//!
//! Cancelling job A releases its work and preserves the runner and job B:
//!
//! ```text
//! Runner process
//! +--------------------------------------------------+
//! | Job A                    Job B                   |
//! | +--------------------+   +--------------------+  |
//! | | Brush execution    |   | Brush execution    |  |
//! | | Background tasks   |   | Background tasks   |  |
//! | | IO and child work  |   | IO and child work  |  |
//! | +--------------------+   +--------------------+  |
//! +--------------------------------------------------+
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
//! Standard utilities behave as GNU's on every Host, macOS included. A script
//! written on a Linux Cloud runs on a Mac unchanged, and a model that writes a
//! BSD form (`sed -i ''`) gets GNU's error for it, which names the problem and
//! is the one models know best.
//!
//! On Windows a job takes drive paths as Windows writes them (`C:/Users/…`),
//! never the Git Bash `/c/…` form, which would name `/c` at the root of the
//! current drive.
//!
//! # Boundaries
//!
//! The crate knows no jobs, execution contexts, services or connection: a
//! declared command reaches the dispatcher only through the `command-sdk`
//! `Handler` the job gives it. It is the only crate that links brush and the
//! standard utilities, and nothing below the executable depends on it, so a
//! change elsewhere recompiles none of them.

mod declared;
mod interpreter;
mod job;
mod process_builtins;
mod scope;
mod signals;
mod timeout;
mod timing;
#[cfg(feature = "testing")]
pub mod testing;
mod utilities;
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
/// a thread of its own, since the pool is far larger than a job can use.
#[derive(Clone)]
pub struct ShellRuntime(tokio::runtime::Handle);

/// Far above the units a job can have: each holds at least one pipe, and a
/// process has at most a few thousand open files.
const UNIT_THREADS: usize = 4096;

impl ShellRuntime {
    /// One worker drives the units' asynchronous parts, such as reaping
    /// children; the blocking pool runs the units themselves.
    pub fn build() -> io::Result<tokio::runtime::Runtime> {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .max_blocking_threads(UNIT_THREADS)
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
