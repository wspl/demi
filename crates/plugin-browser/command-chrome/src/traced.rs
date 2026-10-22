//! Tasks that keep the span of the one that starts them.
//!
//! A conversation's browser logs under the span of the conversation that owns
//! it (`crates/plugin-browser/command/src/conversations.rs`), and every task it
//! starts is spawned through this, so that their events name the conversation
//! too.

use std::future::Future;

use tokio::task::JoinHandle;
use tokio_util::task::TaskTracker;
use tracing::Instrument as _;

/// [`TaskTracker::spawn`], with the current span.
pub trait SpawnTraced {
    fn spawn_traced<F>(&self, task: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static;
}

impl SpawnTraced for TaskTracker {
    fn spawn_traced<F>(&self, task: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.spawn(task.in_current_span())
    }
}
