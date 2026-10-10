//! A test's hold on a flow at one of its steps, for the scenarios that race
//! something against a flow while it waits: the runners' hellos
//! (`HelloStep`) and the pages' synchronization channels (`SyncStep`). From
//! `StepHolds::hold` until the hold is released or dropped, whatever passes
//! the held step waits there.

use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::watch;

/// The step a test holds a flow at, if any.
pub struct StepHolds<S>(Mutex<Option<(S, Arc<Hold>)>>);

impl<S> Default for StepHolds<S> {
    fn default() -> Self {
        Self(Mutex::new(None))
    }
}

impl<S: Copy + PartialEq> StepHolds<S> {
    /// Holds the flow at `step` from now on, until the hold is released.
    pub fn hold(&self, step: S) -> StepHold {
        let hold = Arc::new(Hold {
            released: watch::Sender::new(false),
            arrived: watch::Sender::new(0),
        });
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some((step, hold.clone()));
        StepHold { hold }
    }

    /// Waits while a test holds the flow at `step`.
    pub async fn pass(&self, step: S) {
        let held = match &*self.0.lock().unwrap_or_else(PoisonError::into_inner) {
            Some((held, hold)) if *held == step => Some(hold.clone()),
            _ => None,
        };
        if let Some(hold) = held {
            hold.pass().await;
        }
    }
}

/// A test's hold on a flow at one step. Dropping it releases the flow, so a
/// hold that a failed test leaves behind holds nothing.
pub struct StepHold {
    hold: Arc<Hold>,
}

impl StepHold {
    /// Waits until the flow has reached the held step `count` times,
    /// counting those that went away while they waited there.
    pub async fn until_arrived(&self, count: usize) {
        let mut arrived = self.hold.arrived.subscribe();
        arrived
            .wait_for(|arrived| *arrived >= count)
            .await
            .expect("the hold keeps its count while a test holds it");
    }

    /// Lets what waits through, and everything that comes later.
    pub fn release(self) {
        drop(self);
    }
}

impl Drop for StepHold {
    fn drop(&mut self) {
        self.hold.released.send_replace(true);
    }
}

/// Whether a hold is released, and how many passes reached it.
struct Hold {
    released: watch::Sender<bool>,
    arrived: watch::Sender<usize>,
}

impl Hold {
    /// Counts the pass in and waits until the hold is released.
    async fn pass(&self) {
        self.arrived.send_modify(|arrived| *arrived += 1);
        let mut released = self.released.subscribe();
        // The hold keeps the sender while a pass waits, so the wait ends
        // only with the release.
        let _released = released.wait_for(|released| *released).await;
    }
}

/// Where a hello waits while a test holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelloStep {
    /// At the edge, as the runner's token is looked up, while the edge
    /// watches the runner's socket for its close.
    TokenLookup,
    /// In the shard of the device's owner, which bound the connection and
    /// is about to restore the trees whose commands its runner's records
    /// name.
    TakeUp,
    /// In the shard of the device's owner, which took the socket and has not
    /// bound it to the device yet.
    Bind,
}
