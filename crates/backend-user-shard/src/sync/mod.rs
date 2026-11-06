//! The pages' synchronization channels: the product state and its parts as a
//! channel reads them, and the channel's task in the user's shard.
//!
//! Every change marks the parts it changed in the registry of `core-sync`,
//! after it commits.
//!
//! # Apart from the conversation lanes
//!
//! The channel stays apart from the conversation lanes:
//!
//! - A conversation's frames are a log the page applies in order, so the lane's
//!   outbox keeps every frame and closes a page that falls too far behind.
//! - The channel sends current values, which it can merge, so it never closes a
//!   slow page.
//!
//! On one shared lane, a conversation that lagged would close the page's
//! channel and every other conversation with it, and a burst of command output
//! would hold back the sidebar's changes behind it.
//!
//! The channel carries changes alone. The page reads the product state by
//! `GET /state`, then opens the channel and follows the state from the
//! version the read carries; it applies what the channel sends from then on.

mod channel;
mod state;

pub use self::channel::SyncChannel;
pub use self::state::{StateError, StateReads};
