//! The pages' synchronization channels: the product state and its parts as a
//! channel reads them, and the channel's task in the user's shard.
//!
//! Every change marks the parts it changed in the registry of `backend-page-sync`,
//! after it commits.
//!
//! # Apart from the conversation sockets
//!
//! The channel stays apart from the conversation sockets:
//!
//! - A conversation's frames are a log the page applies in order, so the socket's
//!   outbox keeps every frame and closes a page that falls too far behind.
//! - The channel sends current values, which it can merge, so it never closes a
//!   slow page.
//!
//! On one shared socket, a conversation that lagged would close the page's
//! channel and every other conversation with it, and a burst of command output
//! would hold back the sidebar's changes behind it.
//!
//! The channel carries changes alone. The page reads the product state by
//! `GET /state`, then opens the channel and follows the state from the
//! version the read carries; it applies what the channel sends from then on.

mod channel;
mod state;

use demi_backend_page_sync::Part;

pub use self::channel::ChannelSession;
pub use self::state::{StateError, etag_of, etag_of_json};
use crate::shard::Shard;

impl Shard {
    /// Marks `part` changed on the user's open channels.
    pub fn mark(&self, part: Part) {
        self.services().sync.mark(self.user(), part);
    }
}
