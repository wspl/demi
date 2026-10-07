//! The registry of each user's open synchronization channels, a shared
//! service (`backend.md` § Page synchronization). Whoever commits a change
//! a page shows marks the changed part in it, from any thread: the mark adds
//! the part to the set of each of the user's channels and wakes the
//! channel's task, which reads the part in the shard when it can send it. A
//! channel holds at most one mark per part however far its page falls
//! behind, so it never queues. A part of the backend that follows a user's
//! changes as a page does, the search index, watches them the same way.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use demi_backend_database::accounts::TokenHash;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use tokio::sync::Notify;

/// A part of the product state that a page shows. A channel sends the parts
/// that changed in this order, so a new conversation's summary goes before
/// the order that places it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    /// The conversation's summary.
    Conversation(ConversationId),
    /// The order of every conversation.
    ConversationOrder,
    Preferences,
    User,
    Workspaces,
    Devices,
    Providers,
    Cloud,
    /// The Subagent switch and the subagent profiles.
    Subagents,
    /// The plugin list: whether the user has each plugin on.
    Plugins,
    /// A plugin's state, by the plugin's id.
    Plugin(String),
}

/// Every user's open channels. Cloning it shares it.
#[derive(Clone, Default)]
pub struct SyncRegistry(Arc<Mutex<HashMap<UserId, Vec<Arc<Channel>>>>>);

/// One open channel as the registry marks it.
struct Channel {
    /// The session the channel opened with; none for a watch of the
    /// backend's own, which no sign-out ends.
    session: Option<TokenHash>,
    marked: Mutex<Marked>,
    /// Woken by each mark; a mark while the task is busy leaves a permit, so
    /// none is missed.
    wake: Notify,
}

/// What was marked on a channel since its task last took it.
#[derive(Debug, Default)]
pub struct Marked {
    pub parts: BTreeSet<Part>,
    /// The session the channel opened with was signed out.
    pub session_ended: bool,
}

impl Marked {
    fn is_empty(&self) -> bool {
        self.parts.is_empty() && !self.session_ended
    }
}

impl Channel {
    fn lock(&self) -> MutexGuard<'_, Marked> {
        // A section only inserts into the set or takes it, so a poisoned
        // one is whole.
        self.marked.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn mark(&self, part: Part) {
        self.lock().parts.insert(part);
        self.wake.notify_one();
    }
}

impl SyncRegistry {
    fn lock(&self) -> MutexGuard<'_, HashMap<UserId, Vec<Arc<Channel>>>> {
        // A section only adds, removes or visits channels, so a poisoned map
        // is whole.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers a channel of `user`'s page, opened with the session
    /// `session`, for every change marked from now on, until the
    /// registration is dropped.
    pub fn register(&self, user: &UserId, session: TokenHash) -> Registration {
        self.add(user, Some(session))
    }

    /// Registers a watch of `user`'s changes for a part of the backend that
    /// follows them as a page does, such as the search index, until the
    /// registration is dropped.
    pub fn watch(&self, user: &UserId) -> Registration {
        self.add(user, None)
    }

    fn add(&self, user: &UserId, session: Option<TokenHash>) -> Registration {
        let channel = Arc::new(Channel {
            session,
            marked: Mutex::default(),
            wake: Notify::new(),
        });
        self.lock()
            .entry(user.clone())
            .or_default()
            .push(channel.clone());
        Registration {
            registry: self.clone(),
            user: user.clone(),
            channel,
        }
    }

    /// The marks of `user`'s changes, for a part of the user's shard that
    /// holds no shard.
    pub fn of(&self, user: &UserId) -> UserMarks {
        UserMarks {
            registry: self.clone(),
            user: user.clone(),
        }
    }

    /// Marks `part` changed on each open channel of `user`.
    pub fn mark(&self, user: &UserId, part: Part) {
        if let Some(channels) = self.lock().get(user) {
            for channel in channels {
                channel.mark(part.clone());
            }
        }
    }

    /// Marks `part` changed on every user's open channels, for a change
    /// every user sees, such as a shared instance's provider entry.
    pub fn mark_everyone(&self, part: &Part) {
        for channels in self.lock().values() {
            for channel in channels {
                channel.mark(part.clone());
            }
        }
    }

    /// Ends the open channels of `user` that opened with the session
    /// `session`, which was signed out.
    pub fn end_session(&self, user: &UserId, session: &TokenHash) {
        if let Some(channels) = self.lock().get(user) {
            for channel in channels
                .iter()
                .filter(|channel| channel.session.as_ref() == Some(session))
            {
                channel.lock().session_ended = true;
                channel.wake.notify_one();
            }
        }
    }
}

/// The marks of one user's changes. Cloning it is cheap.
#[derive(Clone)]
pub struct UserMarks {
    registry: SyncRegistry,
    user: UserId,
}

impl UserMarks {
    /// Marks `part` changed on the user's open channels.
    pub fn mark(&self, part: Part) {
        self.registry.mark(&self.user, part);
    }
}

/// A channel's place in the registry, which dropping it gives up.
pub struct Registration {
    registry: SyncRegistry,
    user: UserId,
    channel: Arc<Channel>,
}

impl Registration {
    /// Waits until something is marked on the channel, and takes nothing.
    pub async fn marked(&self) {
        loop {
            if !self.channel.lock().is_empty() {
                return;
            }
            self.channel.wake.notified().await;
        }
    }

    /// Takes what was marked since the last take.
    pub fn take(&self) -> Marked {
        std::mem::take(&mut *self.channel.lock())
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut channels = self.registry.lock();
        if let Some(open) = channels.get_mut(&self.user) {
            open.retain(|channel| !Arc::ptr_eq(channel, &self.channel));
            if open.is_empty() {
                channels.remove(&self.user);
            }
        }
    }
}
