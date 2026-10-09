//! The embedded brush shell and the standard utilities, which run in the
//! runner's process (`runner.md` § Shell jobs): a fresh shell per job, the
//! builtins that act on a process, the declared commands' builtins, which
//! hand each invocation to the handler the job supplies, and the job shell
//! contract's implementation.

mod declared;
mod interpreter;
mod job;
mod process_builtins;
mod scope;
#[cfg(unix)]
mod timeout;
#[cfg(feature = "testing")]
pub mod testing;
mod utilities;
mod work;

use demi_runner_process::job_shell::{JobShell, JobStart, ShellJob};
use futures_util::future::BoxFuture;
use std::{collections::BTreeSet, io};

/// The shell runtime (`concurrency.md` § Runner): every interpreter unit
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
                job.output,
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
