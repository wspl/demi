//! Waiting out a lack of open files.
//!
//! A process keeps the limit the system gives it, and every pipe, local command
//! connection and open file holds one while it lasts. So an operation that
//! finds none left waits and tries again, since another will close, instead of
//! failing. A job's wait ends when the job is cancelled.
//!
//! In the runner this covers:
//!
//! - pipes and connections;
//! - filesystem and working-tree requests;
//! - process and job starts;
//! - everything a job's shell opens (pipes, redirections, here-documents, the
//!   copies it makes for subshells).
//!
//! Nothing tells the process when one closes, so attempts are spaced at most a
//! tenth of a second apart ([`Backoff`]).
//!
//! Running out is never an answer about something else either: a working-tree
//! request does not report a directory as outside a repository because it could
//! not open the repository's files.

use std::{error::Error, future::Future, io, time::Duration};

use tokio_util::sync::CancellationToken;

/// True when `error`, or an error it wraps, means no descriptor was left.
pub fn exhausted(error: &(dyn Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(error) = current {
        if let Some(io) = error.downcast_ref::<io::Error>() {
            if io.kind() == too_many_open_files() {
                return true;
            }
            // `io::Error::other` keeps its payload out of `source`.
            if let Some(inner) = io.get_ref()
                && exhausted(inner)
            {
                return true;
            }
        }
        current = error.source();
    }
    false
}

/// The error a failure for want of an open file stands as, for a copy of one
/// whose cause cannot be copied.
pub fn exhaustion() -> io::Error {
    #[cfg(unix)]
    let code = libc::EMFILE;
    #[cfg(windows)]
    let code = windows_sys::Win32::Foundation::ERROR_TOO_MANY_OPEN_FILES as i32;
    io::Error::from_raw_os_error(code)
}

/// `io::ErrorKind::TooManyOpenFiles` is not stable, so the kind comes from the
/// code the system reports. Wrappers that keep the kind, such as tempfile's
/// path errors, are recognized too.
fn too_many_open_files() -> io::ErrorKind {
    exhaustion().kind()
}

/// Nothing tells a process when a descriptor closes, so attempts are spaced
/// from 5 ms, doubling to at most 100 ms.
pub struct Backoff(Duration);

impl Default for Backoff {
    fn default() -> Self {
        Self(Duration::from_millis(5))
    }
}

impl Backoff {
    /// The pause before the next attempt, for a caller that waits on its own,
    /// such as one on a thread that may block.
    pub fn pause(&mut self) -> Duration {
        #[cfg(feature = "testing")]
        crate::testing::count_pause();
        let pause = self.0;
        self.0 = (self.0 * 2).min(Duration::from_millis(100));
        pause
    }

    pub async fn wait(&mut self) {
        tokio::time::sleep(self.pause()).await;
    }
}

/// Runs `attempt` until it succeeds or fails for another reason. When
/// `cancel` fires during a wait, the last failure is returned.
pub async fn retry<T, E, F, Fut>(cancel: &CancellationToken, mut attempt: F) -> Result<T, E>
where
    E: Error + 'static,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut backoff = Backoff::default();
    loop {
        match attempt().await {
            Err(error) if exhausted(&error) => {
                tokio::select! {
                    _ = cancel.cancelled() => return Err(error),
                    _ = backoff.wait() => {}
                }
            }
            result => return result,
        }
    }
}

/// `retry` for a synchronous call, such as the shell's process and pipe hooks.
/// It runs on a thread that may block, never an async worker, so it sleeps
/// between attempts.
pub fn retry_blocking<T>(mut attempt: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut backoff = Backoff::default();
    loop {
        match attempt() {
            Err(error) if exhausted(&error) => std::thread::sleep(backoff.pause()),
            result => return result,
        }
    }
}
