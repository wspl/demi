//! The pages' synchronization channels (`backend.md` § Browser
//! synchronization, `web-api.md` § Page synchronization): the registry
//! through which every change marks the parts it changed on each user's open
//! channels, the product state and its parts as a channel reads them, and
//! the channel's task in the user's shard.

mod channel;
mod registry;
mod state;

pub(crate) use self::channel::ChannelSession;
pub(crate) use self::registry::{Part, Registration, SyncRegistry, UserMarks};
use crate::shard::Shard;

/// Where a channel waits while a test holds it (`Backend::hold_sync`).
#[cfg(feature = "testing")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStep {
    /// Once it has read its product state, before it sends it.
    Snapshot,
    /// Once a change woke it, before it takes and reads the parts that
    /// changed.
    Changes,
}

impl Shard {
    /// Marks `part` changed on the user's open channels.
    pub(crate) fn mark(&self, part: Part) {
        self.services().sync.mark(self.user(), part);
    }
}
