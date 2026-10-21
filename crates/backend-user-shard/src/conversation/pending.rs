//! The changes an agent asks for that wait for its conversation's tree to be
//! idle: the attached devices it asked to detach.
//!
//! A detach is a transition, which the command's own job and its working tree
//! would always find busy, so the command only records it and answers at once.
//! A rename, an attach and a move apply at once.
//!
//! # Recording
//!
//! - An archive drops it.
//! - An attach of a device cancels its mark to detach; the device stays usable
//!   until the detach is made.
//! - Each change is announced to every node in its next context block, as one
//!   the user makes.
//!
//! # Making the change
//!
//! A watch per conversation waits until the tree does nothing by itself,
//! reserves it in the same step, so nothing else is admitted first, and makes
//! the changes in one transition. A closed tree is idle at once.
//!
//! The changes are stored, so a start makes those a restart cut off.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_host_access::root_of;
use demi_backend_page_sync::Part;
use demi_shared_gates::Reservation;
use demi_web_api_protocol::ids::ConversationId;
use tokio::sync::Notify;
use tokio_util::task::AbortOnDropHandle;

use crate::shard::{Shard, Shards};

/// Each conversation's watch for its tree to be idle.
#[derive(Default)]
pub(crate) struct PendingWatches(RefCell<HashMap<ConversationId, Watch>>);

struct Watch {
    /// Woken when the tree may have gone, which its own watch does not
    /// say while something else holds it.
    poke: Rc<Notify>,
    task: AbortOnDropHandle<()>,
}

impl PendingWatches {
    /// Ends every watch, for the shard's close; what is pending stays
    /// stored for the next start.
    pub(crate) fn clear(&self) {
        self.0.borrow_mut().clear();
    }
}

impl Shard {
    /// Makes the conversation's pending changes once its tree is next idle,
    /// unless a watch for them runs already.
    pub(crate) fn settle_when_idle(&self, id: &ConversationId) {
        if self.is_closing() {
            return;
        }
        let mut watches = self.pending_watches().0.borrow_mut();
        if let Some(watch) = watches.get(id)
            && !watch.task.is_finished()
        {
            watch.poke.notify_one();
            return;
        }
        let poke = Rc::new(Notify::new());
        let task = self.tasks().spawn_local(settle_when_idle(
            Rc::downgrade(&self.this()),
            id.clone(),
            poke.clone(),
        ));
        watches.insert(
            id.clone(),
            Watch {
                poke,
                task: AbortOnDropHandle::new(task),
            },
        );
    }

    /// Tells the conversation's watch, if one runs, that its tree started or
    /// stopped working or was closed.
    pub(crate) fn poke_pending(&self, id: &ConversationId) {
        if let Some(watch) = self.pending_watches().0.borrow().get(id) {
            watch.poke.notify_one();
        }
    }

    /// Makes the pending changes of a conversation a start found: at once
    /// when its tree is closed, before anything opens it; otherwise once it
    /// is idle.
    async fn settle_at_start(&self, id: &ConversationId) {
        if self.agent().tree(&root_of(id)).is_none() && !self.settle(id, None).await {
            return;
        }
        self.settle_when_idle(id);
    }

    /// Makes the conversation's pending detaches with its idle tree reserved,
    /// `tree`, or none when it is closed, in one transition. Answers whether
    /// a detach is left that this one did not see.
    async fn settle(&self, id: &ConversationId, tree: Option<Reservation>) -> bool {
        let control = &self.services().control;
        let host = self.host_shard();
        // The detaches read what they change within the conversation's change
        // boundary, as a user's detach does; its slot is keyed by the id as
        // the record spells it.
        let Ok(spelled) = host.owned_conversation(id).await else {
            return false;
        };
        let _boundary = self.conversations().slot(&spelled.id).changes.acquire().await;
        match host.owned_conversation(id).await {
            Ok(record) if !record.archived => {}
            // An archive dropped the detaches; a gone conversation has none.
            _ => return false,
        }
        let pending = match control.pending_changes(id.clone()).await {
            Ok(pending) if !pending.is_empty() => pending,
            Ok(_) => return false,
            Err(error) => {
                tracing::warn!(conversation = %id, "the pending changes cannot be read: {error}");
                return false;
            }
        };
        let hold = host.hold_when_free(id, tree).await;
        for device in &pending.detaching {
            // Each detach changes the record the next one compares.
            let record = match host.owned_conversation(id).await {
                Ok(record) => record,
                Err(error) => {
                    tracing::warn!(conversation = %id, "the detaches an agent asked for were not made: {error}");
                    break;
                }
            };
            if let Err(refusal) = host.detach(&record, device.clone()).await {
                tracing::warn!(conversation = %id, %device, "an agent's detach was not made: {refusal}");
            }
        }
        drop(hold);
        self.mark(Part::Conversation(id.clone()));
        match control.pending_changes(id.clone()).await {
            Ok(left) => left.detaching.iter().any(|device| !pending.detaching.contains(device)),
            Err(_) => false,
        }
    }
}

/// Waits until the conversation's tree does nothing by itself, or is
/// closed, and makes its pending detaches then; again while a newer one
/// waits.
async fn settle_when_idle(shard: Weak<Shard>, id: ConversationId, poke: Rc<Notify>) {
    loop {
        let Some(live) = shard.upgrade() else {
            return;
        };
        let tree = live.agent().tree(&root_of(&id));
        // The watch starts before the check, so a change after it wakes it.
        let mut watch = tree.as_ref().map(|tree| tree.watch_quiescence());
        // The check and the reservation are one step: no await between them.
        let reserved = match &tree {
            None => Some(None),
            Some(tree) if tree.is_quiescent() => tree.admission().try_reserve().map(Some),
            Some(_) => None,
        };
        drop(tree);
        if let Some(reservation) = reserved {
            if !live.settle(&id, reservation).await {
                return;
            }
            continue;
        }
        drop(live);
        let Some(watch) = watch.as_mut() else {
            continue;
        };
        tokio::select! {
            () = watch.changed() => {}
            () = poke.notified() => {}
        }
    }
}

/// Makes at start the pending changes a restart cut off: each owner's shard
/// makes them at once for a closed tree, before a page opens it.
pub async fn settle_pending(control: &ControlService, shards: &Shards) -> Result<(), StorageError> {
    for (owner, conversation) in control.conversations_pending().await? {
        let settled = shards
            .of(&owner)
            .call(move |shard, _| async move { shard.settle_at_start(&conversation).await })
            .await;
        if settled.is_err() {
            // The backend is shutting down; the next start makes what is left.
            return Ok(());
        }
    }
    Ok(())
}
