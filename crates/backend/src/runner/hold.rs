//! A test's hold on runners' hellos (`runner.md` § Connection and identity),
//! for the scenarios that race a hello against its runner going away and
//! against the backend's shutdown. From `Backend::hold_hellos` until the
//! hold is released or dropped, every hello waits at the step it names.

use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::watch;

/// Where a hello waits while a test holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelloStep {
    /// At the edge, as the runner's token is looked up, while the edge
    /// watches the runner's socket for its close.
    TokenLookup,
    /// In the shard of the device's owner, which took the socket and has not
    /// bound it to the device yet.
    Bind,
}

/// The step a test holds hellos at, if any.
#[derive(Default)]
pub(crate) struct HelloHolds(Mutex<Option<(HelloStep, Arc<Hold>)>>);

impl HelloHolds {
    /// Holds every hello at `step` from now on, until the hold is released.
    pub(crate) fn hold(&self, step: HelloStep) -> HelloHold {
        let hold = Arc::new(Hold {
            released: watch::Sender::new(false),
            arrived: watch::Sender::new(0),
        });
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some((step, hold.clone()));
        HelloHold { hold }
    }

    /// Waits while a test holds hellos at `step`.
    pub(crate) async fn pass(&self, step: HelloStep) {
        let held = match &*self.0.lock().unwrap_or_else(PoisonError::into_inner) {
            Some((held, hold)) if *held == step => Some(hold.clone()),
            _ => None,
        };
        if let Some(hold) = held {
            hold.pass().await;
        }
    }
}

/// A test's hold on hellos at one step. Dropping it releases them, so a
/// hold that a failed test leaves behind holds nothing.
pub struct HelloHold {
    hold: Arc<Hold>,
}

impl HelloHold {
    /// Waits until `count` hellos have reached the held step, including any
    /// that went away while they waited there.
    pub async fn until_arrived(&self, count: usize) {
        let mut arrived = self.hold.arrived.subscribe();
        arrived
            .wait_for(|arrived| *arrived >= count)
            .await
            .expect("the hold keeps its count while a test holds it");
    }

    /// Lets the waiting hellos and every later one through.
    pub fn release(self) {
        drop(self);
    }
}

impl Drop for HelloHold {
    fn drop(&mut self) {
        self.hold.released.send_replace(true);
    }
}

/// Whether a hold is released, and how many hellos reached it.
struct Hold {
    released: watch::Sender<bool>,
    arrived: watch::Sender<usize>,
}

impl Hold {
    /// Counts the hello in and waits until the hold is released.
    async fn pass(&self) {
        self.arrived.send_modify(|arrived| *arrived += 1);
        let mut released = self.released.subscribe();
        // The hold keeps the sender while a hello passes it, so the wait
        // ends only with the release.
        let _released = released.wait_for(|released| *released).await;
    }
}
