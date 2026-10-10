//! The changes an agent asks for that wait for its conversation's tree to
//! be idle (`sessions-and-targets.md` § Switch the primary target, § What the
//! agent can change): its pending move and the attached devices it asked to
//! detach. A watch per conversation waits until the tree does nothing by
//! itself, reserves it in the same step, so nothing else is admitted
//! first, and makes them in one transition; a closed tree is idle at once.
//! A move checks what a user's switch checks, not whether the device is
//! online; one that fails, such as into a project deleted meanwhile, leaves
//! the conversation where it was and wakes the root with Demi's notice of
//! the reason. The changes are stored, so a start
//! makes those a restart cut off.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::conversation_index::{ConversationRecord, ExecutionTarget, PendingMove};
use demi_backend_host_access::root_of;
use demi_backend_host_access::transition::ChangeRefusal;
use demi_web_api_protocol::ids::DeviceId;
use demi_backend_page_sync::Part;
use demi_backend_permissions::PermissionShard;
use demi_shared_gates::Reservation;
use demi_shared_types::{AgentMessage, AgentMessageEvent, BlockId, NodeId};
use demi_web_api_protocol::devices::DeviceKind;
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

    /// Makes the conversation's pending changes with its idle tree reserved,
    /// `tree`, or none when it is closed, in one transition: the move, then
    /// each detach. Answers whether a change is left that this one did not
    /// see, such as a newer move.
    async fn settle(&self, id: &ConversationId, tree: Option<Reservation>) -> bool {
        let control = &self.services().control;
        let host = self.host_shard();
        let record = match host.owned_conversation(id).await {
            Ok(record) if !record.archived => record,
            // An archive dropped the move; a gone conversation has nothing.
            _ => return false,
        };
        let pending = match control.pending_changes(id.clone()).await {
            Ok(pending) if !pending.is_empty() => pending,
            Ok(_) => return false,
            Err(error) => {
                tracing::warn!(conversation = %id, "the pending changes cannot be read: {error}");
                return false;
            }
        };
        let hold = host.hold_when_free(id, tree).await;
        let mut failure = None;
        if let Some(moving) = &pending.moving {
            match self.make_move(&record, moving).await {
                Ok(()) => {}
                Err(reason) => {
                    if let Err(error) = control.drop_pending_move(id.clone(), moving.id.clone()).await {
                        tracing::warn!(conversation = %id, "a failed move was not dropped: {error}");
                    }
                    failure = Some(reason);
                }
            }
        }
        for device in &pending.detaching {
            // The move may have changed the record the detach compares.
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
        if let (Some(reason), Some(moving)) = (failure, &pending.moving) {
            self.tell_move_failed(&record, moving, &reason).await;
        }
        match control.pending_changes(id.clone()).await {
            Ok(left) => {
                left.moving.is_some_and(|newer| Some(&newer.id) != pending.moving.as_ref().map(|moving| &moving.id))
                    || left.detaching.iter().any(|device| !pending.detaching.contains(device))
            }
            Err(_) => false,
        }
    }

    /// Makes the pending move `moving` as a user's switch is made: it checks
    /// what a switch checks, not whether the device is online. The error is
    /// the reason the agent reads.
    async fn make_move(&self, record: &ConversationRecord, moving: &PendingMove) -> Result<(), String> {
        let host = self.host_shard();
        if record.target == moving.target {
            self.services()
                .control
                .drop_pending_move(record.id.clone(), moving.id.clone())
                .await
                .map_err(|error| error.to_string())?;
            return Ok(());
        }
        host.check_destination(&record.owner, &moving.target)
            .await
            .map_err(|refusal| reason(&refusal))?;
        host.switch_target(record, moving.target.clone(), Some(moving.id.clone()))
            .await
            .map_err(|refusal| reason(&refusal))?;
        // A deadline of the old binding never releases the new one.
        self.restart_idle(&record.id);
        Ok(())
    }

    /// Wakes the root with Demi's notice that the move `moving` failed for
    /// `why`, naming where the conversation still runs.
    async fn tell_move_failed(&self, record: &ConversationRecord, moving: &PendingMove, why: &str) {
        let host = self.host_shard();
        let place = match host.resolve_target(record).await {
            Ok(target) => self.place(&target).await,
            Err(_) => "where it was".to_owned(),
        };
        let content = format!(
            "Demi could not make the move this conversation's agent asked for: {why}. The conversation still runs on {place}."
        );
        let id = match BlockId::try_from(format!("move-failed:{}", moving.id)) {
            Ok(id) => id,
            Err(_) => return,
        };
        let at = self.services().clock.now();
        let message = |recipient: &NodeId| AgentMessage {
            id: id.clone(),
            sender: None,
            recipient_id: recipient.clone(),
            timestamp: at,
            content: content.clone(),
            event: AgentMessageEvent::MoveFailed {},
        };
        let root = root_of(&record.id);
        if let Err(error) = PermissionShard::admit(self, &record.id, &root, &message).await {
            tracing::warn!(conversation = %record.id, "a failed move's notice was not delivered: {error}");
        }
    }

    /// Where a target runs, as Demi's messages about a move name it: its
    /// Host and directory, `MacBook Pro (/Users/zan/code/app)`.
    pub(crate) async fn place(&self, target: &ExecutionTarget) -> String {
        format!("{} ({})", self.target_host(target).await, target.path())
    }

    /// The Host a target runs on, by the name Demi's messages give it.
    pub(crate) async fn target_host(&self, target: &ExecutionTarget) -> String {
        match target.device() {
            Some(device) => self.host_name(device).await,
            None => "Cloud".to_owned(),
        }
    }

    /// A device's name, or Cloud for the user's Cloud, as Demi's messages
    /// and commands name it.
    pub(crate) async fn host_name(&self, device: &DeviceId) -> String {
        match self.services().control.device(device.clone()).await {
            Ok(Some(record)) if record.kind == DeviceKind::Managed => "Cloud".to_owned(),
            Ok(Some(record)) => record.name,
            _ => device.to_string(),
        }
    }
}

/// The reason a refused move gives the agent.
fn reason(refusal: &ChangeRefusal) -> String {
    match refusal {
        ChangeRefusal::WorkspaceNotFound => "the project no longer exists".into(),
        ChangeRefusal::DeviceNotFound => "the device is no longer paired".into(),
        ChangeRefusal::Conflict => "another move came first".into(),
        ChangeRefusal::Archived => "the conversation is archived".into(),
        refusal => refusal.to_string(),
    }
}

/// Waits until the conversation's tree does nothing by itself, or is
/// closed, and makes its pending changes then; again while a newer change
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
