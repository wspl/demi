//! A root's failed turn at its retry time.
//!
//! A turn whose failure names when it can succeed again, such as a usage limit
//! that resets at 09:00, stops holding what arrives then: a message or a
//! command report that comes afterwards opens a turn. The turn is resumed then
//! too, as the user's Resume would, when the user's `resumeAtRetry` preference
//! says so at that moment.
//!
//! A usage limit of an entry that rotates its accounts can succeed again as
//! soon as one of them has room, whatever the reset of the account that
//! refused.
//!
//! A message from another conversation is also preceded by a look at whether
//! the failure has passed by then, since the timer's time was read when it
//! was armed, and a tree the message opens learns it only after the message.
//!
//! Each conversation has at most one timer, for the failure that ended its
//! root's last turn.
//!
//! - It is armed as the root comes to rest after that failure, which its phase
//!   change announces.
//! - It is released when the root rests with no failed turn, when it fires, and
//!   when the shard closes.
//! - A tree evicted meanwhile keeps its conversation's timer, which opens the
//!   tree again when it fires.
//! - A timer is the shard's alone: a restart of the backend arms none until the
//!   conversation's tree is open again.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use demi_backend_device_access::root_of;
use demi_shared_gates::Purpose;
use demi_shared_types::{Block, BlockId, ErrorBlock, SessionPhase, Timestamp};
use demi_web_api_protocol::ids::ConversationId;
use tokio_util::task::AbortOnDropHandle;

use super::failure_facts;
use super::failure_facts::rotation_has_room;
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

    /// When the failure `error` can succeed again: now for a usage limit
    /// another account has room for, else as its provider reads its record;
    /// none for a failure that names no time, or one whose provider entry is
    /// gone.
    async fn retry_at(&self, error: ErrorBlock) -> Option<Timestamp> {
        let services = self.services();
        if rotation_has_room(&services.vault, &services.provider_registry, &error).await {
            return Some(services.clock.now());
        }
        let id = error.id.clone();
        let blocks = [Block::Error(error)];
        let facts = failure_facts(&self.services().vault, &self.services().provider_registry, &blocks).await?;
        facts.get(&id)?.retry_at
    }

    /// Before a message from outside the conversation's tree is admitted:
    /// tells a root at rest whose last turn failed that the failure has
    /// passed when a request could succeed now, so the message opens a turn.
    pub(crate) async fn settle_failure_hold(&self, conversation: &ConversationId) {
        let Some(tree) = self.agent().tree(&root_of(conversation)) else {
            return;
        };
        let session = tree.root().session();
        if session.phase() != SessionPhase::Idle {
            return;
        }
        let Some(error) = session.failed_turn() else {
            return;
        };
        let id = error.id.clone();
        let now = self.services().clock.now();
        if self.retry_at(error).await.is_some_and(|at| at <= now) {
            self.failure_passed(conversation, &id);
        }
    }

    /// Tells the conversation's root, when its tree is open, that the failure
    /// of its failed turn whose record is `error` has passed. A tree opened
    /// later is told when its root comes to rest.
    fn failure_passed(&self, conversation: &ConversationId, error: &BlockId) {
        if let Some(tree) = self.agent().tree(&root_of(conversation)) {
            tree.root().session().failure_passed(error);
        }
    }

    /// Resumes the conversation's failed turn whose record is `error`, as the
    /// user's Resume would, when the user's preference says so now.
    async fn resume_failed_turn(&self, conversation: &ConversationId, error: &BlockId) {
        let services = self.services();
        match services.control.preferences(self.user().clone()).await {
            Ok(preferences) if preferences.resume_at_retry == Some(true) => {}
            Ok(_) => {
                tracing::info!(event = "failed_turn_retry.settled", %conversation, turn = %error, outcome = "held", reason = "user_preference", "a failed turn's retry time came; it waits for the user");
                return;
            }
            Err(read) => {
                tracing::warn!(event = "failed_turn_retry.preferences_read_failed", %conversation, turn = %error, error = %read, "the preferences were not read for a failed turn's retry time");
                return;
            }
        }
        match self
            .resume_unfinished_turn(conversation, |failed| &failed.id == error, None)
            .await
        {
            Ok(true) => tracing::info!(event = "failed_turn_retry.settled", %conversation, turn = %error, outcome = "resumed", "a failed turn resumed at its retry time"),
            Ok(false) => tracing::info!(event = "failed_turn_retry.settled", %conversation, turn = %error, outcome = "skipped", "a failed turn was not the conversation's to resume"),
            Err(why) => {
                tracing::warn!(event = "failed_turn_retry.settled", %conversation, turn = %error, outcome = "failed", %why, "a failed turn was not resumed at its retry time");
            }
        }
    }

    /// Resumes the conversation's unfinished turn, as the user's Resume would.
    ///
    /// The conversation's tree opens if it is closed, the backend prepares the
    /// Resume as it prepares the user's, and the root resumes only while it rests
    /// with a last turn whose record `ended` accepts. `reason`, when given, is
    /// what the resume tells the model before "Continue from where you left
    /// off.".
    ///
    /// Answers whether the turn resumed; false when the backend would refuse the
    /// user's Resume, as for an archived conversation, or the last turn did not
    /// end as `ended` expects.
    pub(crate) async fn resume_unfinished_turn(
        &self,
        conversation: &ConversationId,
        ended: impl Fn(&ErrorBlock) -> bool,
        reason: Option<&str>,
    ) -> Result<bool, String> {
        self.restore_tree(conversation).await?;
        let _files = self
            .conversations()
            .slot(conversation)
            .file_gate()
            .enter(Purpose::Demand)
            .await;
        match self.resume_admitted(conversation).await {
            Ok(true) => {}
            Ok(false) => return Ok(false),
            Err(error) => return Err(format!("the resume was not prepared: {error}")),
        }
        let Some(tree) = self.agent().tree(&root_of(conversation)) else {
            return Ok(false);
        };
        let session = tree.root().session();
        let unchanged = session.phase() == SessionPhase::Idle
            && session
                .failed_turn()
                .is_some_and(|failed| ended(&failed));
        if !unchanged {
            return Ok(false);
        }
        // The action reports its course to the conversation's pages as
        // events; a refusal means the session is closing.
        session
            .resume(reason)
            .map_err(|refusal| refusal.to_string())?;
        Ok(true)
    }
}

/// The timer of the failure `error` of `conversation`'s root: it waits
/// until the failure's retry time, then ends the turn's hold and resumes the
/// turn when the user's preference says so. A failure whose time has passed
/// ends the hold at once and resumes nothing; one that names no time waits for
/// nothing.
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
                tracing::info!(event = "failed_turn_retry.scheduled", %conversation, turn = %id, retry_at_ms = retry_at.as_millisecond(), "a failed turn waits for its retry time");
                wait
            }
            _ => {
                live.retry_resumes().take(&conversation, &id);
                live.failure_passed(&conversation, &id);
                return;
            }
        }
    };
    tokio::time::sleep(wait).await;
    let Some(live) = shard.upgrade() else {
        return;
    };
    // Counted among the tree's openers before the timer lets go of its
    // handle, which the shard's close no longer aborts: the close waits for
    // it before the agent shuts down, and it opens nothing once the close
    // began.
    let _opening = live.tree_openers().token();
    live.retry_resumes().take(&conversation, &id);
    if live.is_closing() {
        return;
    }
    live.failure_passed(&conversation, &id);
    live.resume_failed_turn(&conversation, &id).await;
}
