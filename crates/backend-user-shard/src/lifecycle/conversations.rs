//! The conversation idle clock.
//!
//! A conversation that has not been active for the idle window (`super`) has
//! every Host it reaches, primary or attached, told to let go of it.
//!
//! - Its watch starts with the conversation's first Host admission and ends once
//!   it released.
//! - A won target switch starts it again, so a deadline of the old binding never
//!   releases the new one.
//! - An archive ends it.
//!
//! # The release
//!
//! The release is one generic runner message, sent to each connected Host's
//! runner (`crates/backend-host-access/src/access.rs`). Telling takes no file
//! gate beyond the transition's own commit, wakes nothing, stores nothing and
//! waits for no Host. It also goes to the Host a conversation stops using (a
//! target switch, a detach) and to every Host when the conversation is
//! archived or deleted.
//!
//! The runner forwards it to each resident on the device, which ends
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
//! conversation runs there when it is sent: an archive and the idle clock
//! send it only while the backend holds the conversation's file gate, and
//! every job runs in a lease of that gate; a move and a detach send it once
//! the jobs of the conversation on the device they left ended
//! (`crates/backend-host-access/src/transition.rs`). Repeating one is
//! harmless, and it never starts a resident.
//!
//! # What a Host holds
//!
//! The message is an effort, never a record: a runner that is away or loses it
//! keeps what it holds until its next connection, whose hello names the
//! conversations it is to hold (`crates/backend-user-shard/src/shard/adoption.rs`)
//! and has it release every other. A Host is to hold a conversation that is not
//! archived, deleted or detached from it, and that the idle clock did not
//! release; a conversation used on a Host again is held there again, since its
//! next admission starts the watch afresh and takes it off the idle-released
//! ones. The idle-released conversations are kept in memory only: after a
//! restart of the backend a runner told nothing meanwhile is named them again,
//! which releases nothing it still holds for a conversation in use, as a
//! release never starts a resident and the next use starts them.
//!
//! A release that does not end in an answer is logged with the Host and the
//! reason, and changes nothing. A release has no time limit; the connection's
//! own liveness ends it when the Host is gone.
//!
//! An archive, a move and a detach are the user's changes to their own
//! conversation, and none waits for a Host's cleanup, so the watch ends as soon
//! as its release is sent.
//!
//! Forking releases nothing.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use demi_backend_idle_watch::{Activity, IdlePolicy, Retirement};
use demi_web_api_protocol::ids::ConversationId;
use tokio_util::task::AbortOnDropHandle;
use tracing::Instrument as _;

use crate::shard::Shard;
use demi_backend_host_access::root_of;

/// The idle watch of each conversation that reached a Host.
#[derive(Default)]
pub(crate) struct ConversationWatches {
    watches: RefCell<HashMap<ConversationId, AbortOnDropHandle<()>>>,
    /// The conversations whose idle watch released them and that no Host
    /// admission has started again: what the Hosts' `hello` does not name as
    /// held. Not stored; see the module's § What a Host holds.
    released: RefCell<HashSet<ConversationId>>,
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
        self.idle_watches().released.borrow_mut().remove(id);
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
        )
        .instrument(tracing::info_span!("idle_watch", conversation = %id));
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

    /// The conversations the idle clock released and nothing used on a Host
    /// since.
    pub(crate) fn idle_released(&self) -> HashSet<ConversationId> {
        self.idle_watches().released.borrow().clone()
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
                .release_everywhere(&record)
                .await
                .map_err(|error| error.to_string())?;
            shard.idle_watches().released.borrow_mut().insert(id);
            Ok(())
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
