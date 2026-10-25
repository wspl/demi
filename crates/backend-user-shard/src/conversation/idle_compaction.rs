//! A root compacted while idle, shortly before its prompt cache expires.
//!
//! The root's session says, after each answered request, when its idle
//! compaction is due, or that it is not (`IdleCompactionDue`). The shard keeps
//! that time in `idle_compactions` and arms one timer per conversation for it:
//!
//! ```text
//! answered request ──► row written ──► timer sleeps until due_at
//!                                          │
//!   row taken ◄────────────────────────────┘
//!       │
//!       ▼
//!   switch on, lifetime ≥ the user's minimum? ──► tree opened, root compacts
//! ```
//!
//! - A later report replaces the row and the timer; a report of none removes
//!   both, as a compaction's end does.
//! - The time is taken from the row before it acts, so a row an answer wrote
//!   meanwhile, with a later time, keeps its own timer.
//! - The rows outlast a restart: the backend's start opens the shard of each
//!   user who has one, which arms their timers again without opening any
//!   conversation.
//! - A time that came after its cache expired, as for a backend that was down
//!   meanwhile, compacts nothing.
//! - The preferences are read as the time comes, so the switch and the
//!   minimum apply to the times stored before they changed.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use demi_agent_session::IdleCompactionDue;
use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::idle_compactions::IdleCompaction;
use demi_backend_host_access::root_of;
use demi_web_api_protocol::ids::ConversationId;
use tokio_util::task::AbortOnDropHandle;

use crate::shard::{Shard, Shards};

/// Each conversation's timer.
#[derive(Default)]
pub(crate) struct IdleCompactions {
    timers: RefCell<HashMap<ConversationId, Armed>>,
    /// The number of the next timer, which tells a timer that fires whether
    /// it is still its conversation's.
    next: Cell<u64>,
}

struct Armed {
    number: u64,
    task: AbortOnDropHandle<()>,
}

impl IdleCompactions {
    /// Ends every timer, for the shard's close; the rows stay for the next
    /// start.
    pub(crate) fn clear(&self) {
        self.timers.borrow_mut().clear();
    }

    /// Lets go of the timer `number` of `conversation` as it fires, so that it
    /// runs on: dropping its handle would abort it.
    fn release(&self, conversation: &ConversationId, number: u64) {
        let mut timers = self.timers.borrow_mut();
        if timers.get(conversation).is_some_and(|armed| armed.number == number)
            && let Some(armed) = timers.remove(conversation)
        {
            drop(armed.task.detach());
        }
    }
}

impl Shard {
    /// The conversation's root reported when its idle compaction is due:
    /// stores the time and arms its timer, or removes both for none.
    ///
    /// Every stored time has its timer, so a report of none for a
    /// conversation without one, as most answers are, writes nothing.
    pub(crate) fn follow_idle_compaction(&self, conversation: &ConversationId, due: Option<IdleCompactionDue>) {
        if self.is_closing() {
            return;
        }
        if due.is_none() && !self.idle_compactions().timers.borrow().contains_key(conversation) {
            return;
        }
        let due = due.map(|due| IdleCompaction {
            due_at: due.at,
            lifetime: due.lifetime,
        });
        self.arm_idle_compaction(conversation.clone(), due, true);
    }

    /// Arms the conversation's timer for `due`, replacing the one it had,
    /// after writing `due` to its row when `write`.
    fn arm_idle_compaction(&self, conversation: ConversationId, due: Option<IdleCompaction>, write: bool) {
        let compactions = self.idle_compactions();
        let number = compactions.next.get();
        compactions.next.set(number + 1);
        let task = self.tasks().spawn_local(idle_compaction_timer(
            Rc::downgrade(&self.this()),
            conversation.clone(),
            number,
            due,
            write,
        ));
        compactions.timers.borrow_mut().insert(
            conversation,
            Armed {
                number,
                task: AbortOnDropHandle::new(task),
            },
        );
    }

    /// Arms a timer for each of the user's stored times, as the shard opens
    /// at the backend's start.
    async fn arm_idle_compactions(self: Rc<Self>) {
        match self.services().control.idle_compactions(self.user().clone()).await {
            Ok(stored) => {
                for (conversation, due) in stored {
                    self.arm_idle_compaction(conversation, Some(due), false);
                }
            }
            Err(error) => {
                tracing::warn!(event = "idle_compaction.read_failed", user = %self.user(), %error, "the idle compactions' times were not read");
            }
        }
    }

    /// Compacts the conversation's root as its time `due` came, when the
    /// user's preferences say so now. Answers how it settled.
    async fn compact_idle(&self, conversation: &ConversationId, due: IdleCompaction) -> &'static str {
        let control = &self.services().control;
        match control.take_idle_compaction(conversation.clone(), due.due_at).await {
            Ok(true) => {}
            Ok(false) => return "replaced",
            Err(error) => {
                tracing::warn!(event = "idle_compaction.take_failed", %conversation, %error, "an idle compaction's time was not taken");
                return "failed";
            }
        }
        let reported = IdleCompactionDue {
            at: due.due_at,
            lifetime: due.lifetime,
        };
        // A backend that was down past the time finds the cache gone: the
        // pass would pay for the whole history it was to spare.
        if self.services().clock.now() >= reported.expires_at() {
            return "expired";
        }
        let preferences = match control.preferences(self.user().clone()).await {
            Ok(preferences) => preferences,
            Err(error) => {
                tracing::warn!(event = "idle_compaction.preferences_read_failed", %conversation, %error, "the preferences were not read for an idle compaction");
                return "failed";
            }
        };
        if preferences.compact_idle != Some(true) {
            return "switched_off";
        }
        let minimum = preferences.idle_compaction_lifetime.unwrap_or_default().duration();
        if due.lifetime < minimum {
            return "short_lifetime";
        }
        if let Err(why) = self.restore_tree(conversation).await {
            tracing::warn!(event = "idle_compaction.open_failed", %conversation, %why, "a conversation was not opened to compact while idle");
            return "failed";
        }
        let Some(tree) = self.agent().tree(&root_of(conversation)) else {
            return "closed";
        };
        if tree.root().session().compact_idle() {
            "started"
        } else {
            "busy"
        }
    }
}

/// The timer `number` of `conversation`: writes `due` when `write`, then, for
/// a time, waits for it and compacts the root.
async fn idle_compaction_timer(
    shard: Weak<Shard>,
    conversation: ConversationId,
    number: u64,
    due: Option<IdleCompaction>,
    write: bool,
) {
    let wait = {
        let Some(live) = shard.upgrade() else {
            return;
        };
        if write && let Err(error) = live.services().control.set_idle_compaction(conversation.clone(), due).await {
            tracing::warn!(event = "idle_compaction.write_failed", %conversation, %error, "an idle compaction's time was not stored");
        }
        let Some(due) = due else {
            live.idle_compactions().release(&conversation, number);
            return;
        };
        let left = due.due_at.to_jiff().duration_since(live.services().clock.now().to_jiff());
        let wait = Duration::try_from(left).unwrap_or(Duration::ZERO);
        tracing::debug!(event = "idle_compaction.scheduled", %conversation, due_at_ms = due.due_at.as_millisecond(), lifetime_s = due.lifetime.as_secs(), "a conversation compacts while idle at its time");
        wait
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
    live.idle_compactions().release(&conversation, number);
    if live.is_closing() {
        return;
    }
    let Some(due) = due else {
        return;
    };
    let outcome = live.compact_idle(&conversation, due).await;
    tracing::info!(event = "idle_compaction.settled", %conversation, outcome, "an idle compaction's time came");
}

/// Opens at start the shard of each user with a stored idle compaction time,
/// which arms its timers in the background, so the start waits for none.
pub async fn arm_idle_compactions(control: &ControlService, shards: &Shards) -> Result<(), StorageError> {
    for owner in control.idle_compaction_owners().await? {
        let handed = shards.of(&owner).adopt(Shard::arm_idle_compactions).await;
        if handed.is_err() {
            // The backend is shutting down; the rows stay for the next start.
            return Ok(());
        }
    }
    Ok(())
}
