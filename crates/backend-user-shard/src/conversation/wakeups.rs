//! Saved yield wakeups across a backend restart (`runtime.md` § Yield
//! wakeups): the index of conversations keeps when the earliest wakeup each
//! conversation's tree saved is due, which the commits of the tree keep
//! current, and at start the conversation's shard restores the tree at that
//! time with no page attached, so the wakeup fires and its turn runs.

use std::cell::Cell;
use std::rc::{Rc, Weak};
use std::time::Duration;

use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::conversation_index::SavedWakeup;
use demi_backend_database::tree::WakeupDue;
use demi_web_api_protocol::ids::ConversationId;

use crate::shard::{Shard, Shards};

/// The index's record of one conversation's earliest saved wakeup, as one
/// tree store's commits keep it: one write at a time, each of the latest
/// value, so a write that lands late never replaces a newer one.
pub(super) struct IndexedWakeup {
    shard: Weak<Shard>,
    conversation: ConversationId,
    /// What the index holds; none until this store wrote it once.
    indexed: Cell<Option<Option<WakeupDue>>>,
    /// The conversation's earliest wakeup after the latest commit.
    latest: Cell<Option<WakeupDue>>,
    /// Whether a write is under way, which writes the latest value next.
    writing: Cell<bool>,
}

impl IndexedWakeup {
    pub(super) fn new(shard: Weak<Shard>, conversation: ConversationId) -> Rc<Self> {
        Rc::new(Self {
            shard,
            conversation,
            indexed: Cell::new(None),
            latest: Cell::new(None),
            writing: Cell::new(false),
        })
    }

    /// Takes the conversation's earliest wakeup after a commit, and writes
    /// it into the index as a task of the shard unless the index holds it.
    pub(super) fn committed(self: &Rc<Self>, wakeup: Option<WakeupDue>) {
        self.latest.set(wakeup);
        if self.writing.get() || self.indexed.get() == Some(wakeup) {
            return;
        }
        // A shard that is gone has no tree left to commit.
        let Some(shard) = self.shard.upgrade() else {
            return;
        };
        self.writing.set(true);
        let this = self.clone();
        let tasks = shard.tasks().clone();
        tasks.spawn_local(async move {
            this.write(&shard.services().control).await;
        });
    }

    /// Writes the latest value until the index holds it. A failed write is
    /// logged and leaves the record as it was: the next commit writes again.
    async fn write(&self, control: &ControlService) {
        loop {
            let wakeup = self.latest.get();
            if self.indexed.get() == Some(wakeup) {
                break;
            }
            match control.set_wakeup(self.conversation.clone(), wakeup).await {
                Ok(()) => self.indexed.set(Some(wakeup)),
                Err(error) => {
                    tracing::warn!(
                        conversation = %self.conversation,
                        error = &error as &dyn std::error::Error,
                        "the conversation's saved wakeup was not indexed"
                    );
                    break;
                }
            }
        }
        self.writing.set(false);
    }
}

/// Arms each saved wakeup again at start: every conversation the index lists
/// with one, archived ones aside, has its owner's shard restore its tree
/// when the wakeup is due. Answers once each shard has its task.
pub async fn rearm_wakeups(control: &ControlService, shards: &Shards) -> Result<(), StorageError> {
    for SavedWakeup {
        conversation,
        owner,
        due,
    } in control.saved_wakeups().await?
    {
        let handed = shards
            .of(&owner)
            .adopt(move |shard| shard.restore_when_due(conversation, due))
            .await;
        if handed.is_err() {
            // The backend is shutting down; the next start arms what is left.
            return Ok(());
        }
    }
    Ok(())
}

impl Shard {
    /// Waits until `due`, then restores the conversation's tree with no page
    /// attached, which arms its saved wakeups again; a wakeup due at start
    /// restores it at once. The wait ends with the shard's close.
    async fn restore_when_due(self: Rc<Self>, conversation: ConversationId, due: WakeupDue) {
        if let WakeupDue::At(at) = due {
            let wait = at.as_millisecond() - self.services().clock.now().as_millisecond();
            let wait = Duration::from_millis(u64::try_from(wait).unwrap_or(0));
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                () = self.closed() => return,
            }
        }
        // Counted before the restore, so the close waits for it before the
        // agent shuts down; one that would start as the close begins does not.
        let _opening = self.tree_openers().token();
        if self.is_closing() {
            return;
        }
        if let Err(why) = self.restore_tree(&conversation).await {
            tracing::warn!(%conversation, "the tree of a saved wakeup was not restored: {why}");
        }
    }
}
