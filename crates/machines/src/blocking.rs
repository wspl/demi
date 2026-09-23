//! The ways off the loop for blocking work (`concurrency.md` § Blocking
//! work): the blocking pool for file sequences and syncs, mounts, loop and
//! freeze ioctls, copies, hashing and firewall updates, and a dedicated
//! thread for work that must not run on a pooled one. Every function that
//! blocks takes an [`OffLoop`], and only this module makes one, so calling
//! such a function on the loop does not compile.

/// Proof that the caller runs on a blocking thread, not on the loop.
pub struct OffLoop {
    _private: (),
}

impl OffLoop {
    /// A token for a test that runs blocking functions on its own thread.
    #[cfg(test)]
    pub(crate) fn in_test() -> Self {
        Self { _private: () }
    }
}

/// Runs `job` on the blocking pool and returns its result. A panic in the
/// job continues on the caller. A job is never cancelled: the loop awaits it
/// to its end, so a sequence of file operations is never cut short.
pub async fn run<T: Send + 'static>(job: impl FnOnce(&OffLoop) -> T + Send + 'static) -> T {
    let task = tokio::task::spawn_blocking(move || job(&OffLoop { _private: () }));
    match task.await {
        Ok(value) => value,
        Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        // A blocking job is cancelled only when the runtime shuts down before
        // it starts, and the loop that awaits it shuts down with the runtime.
        Err(error) => panic!("a blocking job did not run: {error}"),
    }
}

/// Runs `job` on a thread of its own, which ends when the job returns: for
/// work that changes its thread, such as entering another namespace, which
/// a pooled thread would carry into later jobs.
pub async fn run_on_new_thread<T: Send + 'static>(
    name: &str,
    job: impl FnOnce(&OffLoop) -> std::io::Result<T> + Send + 'static,
) -> std::io::Result<T> {
    let (done, result) = tokio::sync::oneshot::channel();
    // The thread reports through `done` and ends right after, so nothing
    // joins it.
    std::thread::Builder::new().name(name.to_owned()).spawn(move || {
        let outcome = job(&OffLoop { _private: () });
        // The caller may have stopped waiting; then no one needs the outcome.
        let _ = done.send(outcome);
    })?;
    result
        .await
        .map_err(|_| std::io::Error::other("a thread ended without a result"))?
}
