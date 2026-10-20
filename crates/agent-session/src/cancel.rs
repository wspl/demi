//! Stopping a running action.
//!
//! An action has one token and a reason it was cancelled. It also has an
//! acknowledgement that it recorded the stop, which `abort()` waits for and
//! which says whether another `abort` would then have stopped more.
//!
//! Each `abort` stops one thing, in this order:
//!
//! 1. The running action.
//! 2. Otherwise the first waiting action.
//! 3. Otherwise nothing.
//!
//! The stopped action records the stop itself, because a cancelled provider run
//! ends without an event. The stop is acknowledged only once the record is in
//! the transcript. An action that was saving a history rewrite records it after
//! the rewrite is published, so a rewrite never loses a stop.
//!
//! A Stop stops only what the node does at that moment and holds nothing
//! afterwards:
//!
//! - Its subagents run on.
//! - An agent message or a command report that arrives later wakes the node as
//!   it wakes an idle one.

use std::{cell::RefCell, future::Future, rc::Rc};

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::TurnError;

/// Why an action was stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CancelReason {
    /// The user's Stop: the action leaves the stopped marker.
    Stop,
    /// A hold of the conversation, such as a Cloud reset or a deletion,
    /// stops it as the user's Stop does; the reason says which, as its
    /// interrupted calls name it (`crates/agent-session/src/core.rs`).
    Hold(Rc<str>),
    /// The session is being disposed: the action leaves the interruption
    /// record.
    Shutdown,
}

pub(crate) struct TurnCancel {
    token: CancellationToken,
    /// The first reason wins.
    reason: RefCell<Option<CancelReason>>,
    /// Set once the action recorded how it ended: whether another `abort`
    /// would stop something then.
    recorded: watch::Sender<Option<bool>>,
}

impl TurnCancel {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            token: CancellationToken::new(),
            reason: RefCell::new(None),
            recorded: watch::Sender::new(None),
        })
    }

    pub(crate) fn cancel(&self, reason: CancelReason) {
        self.reason.borrow_mut().get_or_insert(reason);
        self.token.cancel();
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    pub(crate) fn reason(&self) -> Option<CancelReason> {
        self.reason.borrow().clone()
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
