//! The lock installers of one artifact share between processes.

use std::{fs::TryLockError, path::Path, time::Duration};

use tokio_util::sync::CancellationToken;

use crate::Error;

/// How often a waiting installer tries the lock again.
const RETRY: Duration = Duration::from_millis(50);

/// An exclusive OS lock on a file beside an installation, held until dropped.
/// Every process installing the same artifact takes it first, so one
/// downloads and the others find its result.
#[derive(Debug)]
pub struct InstallLock {
    /// Closing the file releases the lock.
    _file: std::fs::File,
}

impl InstallLock {
    /// Waits for the lock on `path`, creating the file; cancellation gives
    /// up the wait.
    pub async fn acquire(path: &Path, cancel: &CancellationToken) -> Result<Self, Error> {
        let file = std::fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(TryLockError::WouldBlock) => {
                    tokio::select! {
                        _ = cancel.cancelled() => return Err(Error::Cancelled),
                        _ = tokio::time::sleep(RETRY) => {}
                    }
                }
                Err(TryLockError::Error(error)) => return Err(error.into()),
            }
        }
    }
}
