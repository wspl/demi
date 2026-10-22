//! The conversation idle clock.
//!
//! A conversation that has not been active for the idle window (`super`) hears
//! the conversation release on every Host it reaches, primary or attached, whose
//! runner is connected.
//!
//! - Its watch starts with the conversation's first Host admission and ends once
//!   it released.
//! - A won target switch starts it again, so a deadline of the old binding never
//!   releases the new one.
//! - An archive ends it.
//!
//! # The release
//!
//! The release is one generic runner message, sent through the conversation's
//! host access in its lifecycle form, which takes no file gate and wakes
//! nothing. It also goes to the Host a conversation stops using (a target
//! switch, a detach) and to every Host when the conversation is archived or
//! deleted.
//!
//! The runner forwards it to each resident service on the device, which ends
//! whatever it holds for the conversation (the browser closes its Chrome and
//! removes its profile).
//!
//! It says whether the conversation is over: an archive and a deletion say so,
//! the idle clock, a target switch and a detach do not. The runner removes the
//! conversation's recordings only when it is, and a runner that was offline
//! then learns it from its hello, which names the archived conversations
//! (`crates/runner-jobs/src/scratch.rs`).
//!
//! It can be sent without checking what runs there because no job of the
//! conversation runs there when it arrives: an archive, a detach and the idle
//! clock send it only while the backend holds the conversation's file gate,
//! and every job runs in a lease of that gate; a move sends it once the jobs
//! of the conversation on the device it left ended
//! (`crates/backend-host-access/src/transition.rs`). Repeating one is
//! harmless, and it never starts a service.
//!
//! It is never sent to a stopped Cloud or an offline device, nor later: the stop
//! or the connection loss already ended every service's state.
//!
//! # A failed release
//!
//! A release that fails is logged with the Host and the reason, fails nothing
//! that sent it and is not sent again, because nothing is left for it. A release
//! fails when:
//!
//! - the connection is lost before the answer;
//! - a service could not end what it held;
//! - no answer comes within six minutes.
//!
//! The cases need no second attempt:
//!
//! - A runner that lost its connection has ended every service's state.
//! - A service that failed is retired and so ends what it held.
//! - One that answers too late finishes all the same.
//!
//! An archive, a move and a detach are the user's changes to their own
//! conversation, and failing one because a Host's cleanup failed would leave a
//! refusal to retry that changes nothing on the Host. So the watch ends after its
//! release however a Host answered.
//!
//! Forking releases nothing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use demi_backend_idle_watch::{Activity, IdlePolicy, Retirement};
use demi_web_api_protocol::ids::ConversationId;
use tokio_util::task::AbortOnDropHandle;

use crate::shard::Shard;
use demi_backend_host_access::root_of;

/// The idle watch of each conversation that reached a Host.
#[derive(Default)]
pub(crate) struct ConversationWatches {
    watches: RefCell<HashMap<ConversationId, AbortOnDropHandle<()>>>,
}

impl Shard {
    /// What the conversation is doing: a turn of its tree, an operation
    /// holding its file gate, or a user stream someone has open.
    pub(crate) fn conversation_activity(&self, id: &ConversationId) -> Activity {
        let slot = self.conversations().slot(id);
        let host = Activity::of(&slot.file_gate().state()).and(Activity::of(&slot.streams.state()));
        match self.agent().tree(&root_of(id)) {
            Some(tree) => host.and(Activity::of(&tree.admission().state())),
            None => host,
        }
    }

    /// Whether the conversation's idle watch runs: from its first Host
    /// admission until it released the conversation or was ended.
    pub(crate) fn tracks_idle(&self, id: &ConversationId) -> bool {
        self.idle_watches()
            .watches
            .borrow()
            .get(id)
            .is_some_and(|watch| !watch.is_finished())
    }

    /// Starts the conversation's idle watch unless one runs, as each Host
    /// admission does.
    pub(crate) fn track_idle(&self, id: &ConversationId) {
        if self.is_closing() || self.tracks_idle(id) {
            return;
        }
        let policy = ConversationIdle {
            shard: Rc::downgrade(&self.this()),
            id: id.clone(),
        };
        let lifecycle = &self.services().lifecycle;
        let watch = demi_backend_idle_watch::watch(
            policy,
            lifecycle.idle_window,
            lifecycle.idle_poll,
            self.tasks().clone(),
        );
        let watch = AbortOnDropHandle::new(self.tasks().spawn_local(watch));
        self.idle_watches()
            .watches
            .borrow_mut()
            .insert(id.clone(), watch);
    }

    /// Starts the conversation's idle watch again, as a won target switch
    /// does.
    pub(crate) fn restart_idle(&self, id: &ConversationId) {
        self.stop_idle(id);
        self.track_idle(id);
    }

    /// Ends the conversation's idle watch, as an archive does; a release
    /// already running finishes.
    pub(crate) fn stop_idle(&self, id: &ConversationId) {
        self.idle_watches().watches.borrow_mut().remove(id);
    }

    /// Ends every conversation's idle watch, for the shard's close.
    pub(crate) fn stop_idle_watches(&self) {
        self.idle_watches().watches.borrow_mut().clear();
    }
}

/// The idle rule for a conversation on its paired devices.
struct ConversationIdle {
    shard: Weak<Shard>,
    id: ConversationId,
}

impl ConversationIdle {
    fn shard(&self) -> Result<Rc<Shard>, String> {
        self.shard
            .upgrade()
            .ok_or_else(|| "the shard is gone".to_owned())
    }
}

impl IdlePolicy for ConversationIdle {
    async fn check(&self) -> Result<Activity, String> {
        Ok(self.shard()?.conversation_activity(&self.id))
    }

    async fn reserve(&self) -> Result<Option<Retirement>, String> {
        let shard = self.shard()?;
        let tree = match shard.agent().tree(&root_of(&self.id)) {
            Some(tree) => match tree.admission().try_reserve() {
                Some(reservation) => Some(reservation),
                None => return Ok(None),
            },
            None => None,
        };
        let Some(files) = shard
            .conversations()
            .slot(&self.id)
            .file_gate()
            .try_reserve()
        else {
            return Ok(None);
        };
        let id = self.id.clone();
        Ok(Some(Box::pin(async move {
            let _held = (files, tree);
            let record = shard
                .host_shard()
                .owned_conversation(&id)
                .await
                .map_err(|error| error.to_string())?;
            shard
                .host_shard()
                .release_everywhere(&record, false)
                .await
                .map_err(|error| error.to_string())
        })))
    }

    async fn changed(&self) {
        let Ok(shard) = self.shard() else {
            return std::future::pending().await;
        };
        let mut files = shard.conversations().slot(&self.id).file_gate().subscribe();
        // The slot keeps the gate's sender while the shard lives.
        let _ = files.changed().await;
    }
}
