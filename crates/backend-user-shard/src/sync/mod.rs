//! The pages' synchronization channels: the product state and its parts as a
//! channel reads them, and the channel's task in the user's shard. Every change
//! marks the parts it changed in the registry of `backend-page-sync`, after it
//! commits.
//!
//! The channel stays apart from the conversation sockets. A conversation's frames
//! are a log the page applies in order, so the socket's outbox keeps every frame
//! and closes a page that falls too far behind; the channel sends current values,
//! which it can merge, so it never closes a slow page. On one shared socket, a
//! conversation that lagged would close the page's channel and every other
//! conversation with it, and a transcript's handshake or a burst of command output
//! would hold back the sidebar's changes behind it.

mod channel;
mod state;

use demi_backend_page_sync::Part;

pub use self::channel::ChannelSession;
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
    /// Once it has read the providers, in the product state or as a part.
    Providers,
    /// Once it has noted what its page says it shows.
    Showing,
}

impl Shard {
    /// Marks `part` changed on the user's open channels.
    pub fn mark(&self, part: Part) {
        self.services().sync.mark(self.user(), part);
    }
}
