//! The manager-wide gate (`concurrency.md` § Machine manager): device
//! operations share it, `reconcile` and shutdown take it whole. It is first
//! in, first out, so an exclusive entrant waits for the operations in flight
//! and every later request waits behind it.

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

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc, time::Duration};

    use super::*;

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn exclusive_entry_waits_for_shared_work_and_holds_back_later_entrants() {
        let admission = Rc::new(Admission::new());
        let order = Rc::new(RefCell::new(Vec::new()));
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let running = admission.enter().await.expect("open");
                let exclusive = tokio::task::spawn_local({
                    let admission = admission.clone();
                    let order = order.clone();
                    async move {
                        let _whole = admission.exclusive().await.expect("open");
                        order.borrow_mut().push("exclusive");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        order.borrow_mut().push("exclusive done");
                    }
                });
                tokio::task::yield_now().await;
                let later = tokio::task::spawn_local({
                    let admission = admission.clone();
                    let order = order.clone();
                    async move {
                        let _shared = admission.enter().await.expect("open");
                        order.borrow_mut().push("later");
                    }
                });
                tokio::time::sleep(Duration::from_secs(5)).await;
                assert!(order.borrow().is_empty(), "{:?}", order.borrow());
                drop(running);
                exclusive.await.expect("exclusive");
                later.await.expect("later");
                assert_eq!(*order.borrow(), ["exclusive", "exclusive done", "later"]);
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_closed_gate_refuses_waiting_and_new_entrants() {
        let admission = Rc::new(Admission::new());
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let whole = admission.exclusive().await.expect("open");
                let waiting = tokio::task::spawn_local({
                    let admission = admission.clone();
                    async move { admission.enter().await.map(drop) }
                });
                tokio::task::yield_now().await;
                admission.close();
                drop(whole);
                assert!(waiting.await.expect("waiting").is_err());
                assert!(admission.enter().await.is_err());
            })
            .await;
    }
}
