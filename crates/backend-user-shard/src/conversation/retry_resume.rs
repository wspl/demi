//! A root's failed turn resumed at its retry time
//! (`failures-and-recovery.md` § The unfinished turn): a turn whose failure
//! names when it can succeed again, such as a usage limit that resets at
//! 09:00, is resumed then, as the user's Resume would, when the user's
//! `resumeAtRetry` preference says so at that moment.
//!
//! Each conversation has at most one timer, for the failure that ended its
//! root's last turn. It is armed as the root comes to rest after that
//! failure, which its phase change announces, and released when the root
//! rests with no failed turn, when it fires, and when the shard closes. A
//! tree evicted meanwhile keeps its conversation's timer, which opens the
//! tree again when it fires. A timer is the shard's alone: a restart of the
//! backend arms none until the conversation's tree is open again.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use demi_backend_host_access::root_of;
use demi_shared_gates::Purpose;
use demi_shared_types::{Block, BlockId, ErrorBlock, SessionPhase, Timestamp};
use demi_web_api_protocol::ids::ConversationId;
use tokio_util::task::AbortOnDropHandle;

use super::failure_facts;
use crate::shard::Shard;

/// Each conversation's timer.
#[derive(Default)]
pub(crate) struct RetryResumes(RefCell<HashMap<ConversationId, Armed>>);

/// A timer for the failure `error`.
struct Armed {
    error: BlockId,
    task: AbortOnDropHandle<()>,
}

impl RetryResumes {
    /// Ends every timer, for the shard's close.
    pub(crate) fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    /// Takes the timer of `conversation` out when it is the one for
    /// `error`, so that it runs on as it fires: dropping its entry would
    /// abort it.
    fn take(&self, conversation: &ConversationId, error: &BlockId) {
        let mut timers = self.0.borrow_mut();
        if timers
            .get(conversation)
            .is_some_and(|armed| &armed.error == error)
            && let Some(armed) = timers.remove(conversation)
        {
            // The task is the caller; its handle is let go, not aborted.
            drop(armed.task.detach());
        }
    }
}

impl Shard {
    /// The conversation's root changed its phase, or its tree opened or
    /// went: a live root at rest whose last turn failed has a timer for that
    /// failure, and one with no failed turn has none. A root that works
    /// leaves the timer as it is, since its action's end decides.
    pub(crate) fn follow_failed_turn(&self, conversation: &ConversationId) {
        if self.is_closing() {
            return;
        }
        let Some(tree) = self.agent().tree(&root_of(conversation)) else {
            return;
        };
        let session = tree.root().session();
        if session.phase() != SessionPhase::Idle {
            return;
        }
        let failed = session.failed_turn();
        let mut timers = self.retry_resumes().0.borrow_mut();
        let Some(error) = failed else {
            timers.remove(conversation);
            return;
        };
        if timers
            .get(conversation)
            .is_some_and(|armed| armed.error == error.id)
        {
            return;
        }
        let id = error.id.clone();
        let task = self.tasks().spawn_local(resume_at_retry(
            Rc::downgrade(&self.this()),
            conversation.clone(),
            error,
        ));
        timers.insert(
            conversation.clone(),
            Armed {
                error: id,
                task: AbortOnDropHandle::new(task),
            },
        );
    }

    /// When the failure `error` can succeed again, as its provider reads
    /// its record; none for a failure that names no time, or one whose
    /// provider entry is gone.
    async fn retry_at(&self, error: ErrorBlock) -> Option<Timestamp> {
        let id = error.id.clone();
        let blocks = [Block::Error(error)];
        let facts = failure_facts(&self.services().assembly, &blocks).await?;
        facts.get(&id)?.retry_at
    }

    /// Resumes the conversation's failed turn whose record is `error`, as the
    /// user's Resume would, when the user's preference says so now: the
    /// conversation's tree opens if it is closed, the backend prepares the
    /// Resume as it prepares the user's, and the root resumes only while it
    /// rests with that failure as its last turn's record.
    async fn resume_failed_turn(&self, conversation: &ConversationId, error: &BlockId) {
        let services = self.services();
        match services.control.preferences(self.user().clone()).await {
            Ok(preferences) if preferences.resume_at_retry == Some(true) => {}
            Ok(_) => {
                tracing::info!(%conversation, "a failed turn's retry time came; it waits for the user, as their preference says");
                return;
            }
            Err(error) => {
                tracing::warn!(%conversation, "the preferences were not read for a failed turn's retry time: {error}");
                return;
            }
        }
        if let Err(why) = self.restore_tree(conversation).await {
            tracing::warn!(%conversation, "a failed turn was not resumed at its retry time: {why}");
            return;
        }
        let _files = self
            .conversations()
            .slot(conversation)
            .file_gate()
            .enter(Purpose::Demand)
            .await;
        let reason = match self.resume_frame_reason(conversation).await {
            Ok(Some(reason)) => reason,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(%conversation, "a failed turn's resume was not prepared: {error}");
                return;
            }
        };
        let Some(tree) = self.agent().tree(&root_of(conversation)) else {
            return;
        };
        let session = tree.root().session();
        let unchanged = session.phase() == SessionPhase::Idle
            && session
                .failed_turn()
                .is_some_and(|failed| &failed.id == error);
        if !unchanged {
            return;
        }
        // The action reports its course to the conversation's pages as
        // events; a refusal means the session is closing.
        match session.resume(reason.as_deref()) {
            Ok(_) => tracing::info!(%conversation, "a failed turn resumed at its retry time"),
            Err(refusal) => {
                tracing::warn!(%conversation, "a failed turn was not resumed at its retry time: {refusal}");
            }
        }
    }
}

/// The timer of the failure `error` of `conversation`'s root: it waits
/// until the failure's retry time, then resumes the turn when the user's
/// preference says so. A failure that names no time, or one that has
/// passed, waits for nothing.
async fn resume_at_retry(shard: Weak<Shard>, conversation: ConversationId, error: ErrorBlock) {
    let id = error.id.clone();
    let wait = {
        let Some(live) = shard.upgrade() else {
            return;
        };
        let Some(retry_at) = live.retry_at(error).await else {
            live.retry_resumes().take(&conversation, &id);
            return;
        };
        let left = retry_at
            .to_jiff()
            .duration_since(live.services().clock.now().to_jiff());
        match Duration::try_from(left) {
            Ok(wait) if !wait.is_zero() => {
                tracing::info!(%conversation, %retry_at, "a failed turn waits for its retry time");
                wait
            }
            _ => {
                live.retry_resumes().take(&conversation, &id);
                return;
            }
        }
    };
    tokio::time::sleep(wait).await;
    let Some(live) = shard.upgrade() else {
        return;
    };
    live.retry_resumes().take(&conversation, &id);
    live.resume_failed_turn(&conversation, &id).await;
}
