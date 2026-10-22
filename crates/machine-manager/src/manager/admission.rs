//! The manager-wide gate: device operations share it, `reconcile` and
//! shutdown take it whole. It is first in, first out, so an exclusive entrant
//! waits for the operations in flight and every later request waits behind
//! it.

use tokio::sync::{AcquireError, Semaphore, SemaphorePermit};

/// Every permit; an exclusive entrant takes them all.
const PERMITS: u32 = u32::MAX;

pub struct Admission {
    semaphore: Semaphore,
}

/// The gate refuses work: the manager is stopping.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("the Cloud manager is stopping")]
pub struct Closed;

impl From<AcquireError> for Closed {
    fn from(_: AcquireError) -> Self {
        Self
    }
}

impl Admission {
    pub fn new() -> Self {
        Self {
            semaphore: Semaphore::new(PERMITS as usize),
        }
    }

    /// Admits one device operation beside others.
    pub async fn enter(&self) -> Result<SemaphorePermit<'_>, Closed> {
        Ok(self.semaphore.acquire().await?)
    }

    /// Admits work that needs every device operation finished.
    pub async fn exclusive(&self) -> Result<SemaphorePermit<'_>, Closed> {
        Ok(self.semaphore.acquire_many(PERMITS).await?)
    }

    /// Refuses every entrant from now on, the waiting ones too, so no request
    /// accepted before shutdown can start a sandbox after the drain.
    pub fn close(&self) {
        self.semaphore.close();
    }
}

impl Default for Admission {
    fn default() -> Self {
        Self::new()
    }
}
