//! Stopping a running action (`runtime.md` § Stop, § Dispose and restore):
//! one token per action, why it was cancelled, and the acknowledgement that
//! the action recorded the stop, which `abort()` waits for and which says
//! whether another `abort` would then have stopped more.

use std::{cell::Cell, future::Future, rc::Rc};

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::TurnError;

/// Why an action was stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CancelReason {
    /// The user's Stop: the action leaves the stopped marker.
    Stop,
    /// The session is being disposed: the action leaves the interruption
    /// record.
    Shutdown,
}

pub(crate) struct TurnCancel {
    token: CancellationToken,
    /// The first reason wins.
    reason: Cell<Option<CancelReason>>,
    /// Set once the action recorded how it ended: whether another `abort`
    /// would stop something then.
    recorded: watch::Sender<Option<bool>>,
}

impl TurnCancel {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            token: CancellationToken::new(),
            reason: Cell::new(None),
            recorded: watch::Sender::new(None),
        })
    }

    pub(crate) fn cancel(&self, reason: CancelReason) {
        if self.reason.get().is_none() {
            self.reason.set(Some(reason));
        }
        self.token.cancel();
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    pub(crate) fn reason(&self) -> Option<CancelReason> {
        self.reason.get()
    }

    /// Fails with [`TurnError::Cancelled`] once the action is stopped; every
    /// step that is not raced checks this after it.
    pub(crate) fn check(&self) -> Result<(), TurnError> {
        if self.token.is_cancelled() {
            return Err(TurnError::Cancelled);
        }
        Ok(())
    }

    /// Runs `future` unless the action is stopped first, in which case the
    /// future is dropped: it must be safe to drop at any await.
    pub(crate) async fn guard<F: Future>(&self, future: F) -> Result<F::Output, TurnError> {
        self.token
            .run_until_cancelled(future)
            .await
            .ok_or(TurnError::Cancelled)
    }

    /// A token that a provider run or a tool call observes.
    pub(crate) fn child_token(&self) -> CancellationToken {
        self.token.child_token()
    }

    /// The action recorded how it ended; `can_abort_again` says whether
    /// another `abort` would stop something at that moment.
    pub(crate) fn acknowledge(&self, can_abort_again: bool) {
        self.recorded.send_replace(Some(can_abort_again));
    }

    /// Waits until the action recorded how it ended, and answers whether
    /// another `abort` would then have stopped something.
    pub(crate) async fn recorded(&self) -> bool {
        let mut recorded = self.recorded.subscribe();
        // The sender lives in `self`, which outlives this wait, so the wait
        // ends only when the action acknowledged.
        match recorded.wait_for(Option::is_some).await {
            Ok(again) => *again == Some(true),
            Err(_) => false,
        }
    }
}
