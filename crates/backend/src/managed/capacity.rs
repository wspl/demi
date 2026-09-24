//! Cloud capacity (`managed-hosts.md` § Lifecycle and capacity): the one
//! Cloud fact that spans users, how many machines are not stopped. A Cloud
//! takes a permit when it leaves the stopped state, to boot or to reset,
//! and gives it back only when it is stopped again; a Cloud that finds none
//! free fails to start, and nothing queues for one.

use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// The permits of every user's Cloud. `Send + Sync`; cloning shares them.
#[derive(Clone)]
pub(crate) struct CloudCapacity(Arc<Semaphore>);

/// One Cloud's place among the machines that are not stopped; dropping it
/// gives the place back.
pub(crate) struct CapacityPermit(#[allow(dead_code, reason = "held for its drop")] OwnedSemaphorePermit);

impl CloudCapacity {
    /// Capacity for `machines` Clouds that are not stopped.
    pub(crate) fn new(machines: usize) -> Self {
        Self(Arc::new(Semaphore::new(machines)))
    }

    /// A permit, now, or none while every one is taken.
    pub(crate) fn try_take(&self) -> Option<CapacityPermit> {
        self.0.clone().try_acquire_owned().ok().map(CapacityPermit)
    }
}
