//! The page connection: the one WebSocket a page holds to the backend,
//! `WS /api/connect?page=<page id>`, and the lanes it carries.
//!
//! A lane carries what one of the page's parts says with the backend: the
//! synchronization channel, a conversation's lane, a file watch, a device's
//! signaling or a user stream. Each lane's messages are its protocol's, as
//! they would be on a WebSocket of its own; the connection only numbers,
//! cuts and paces them, in the frames of `crates/shared-lanes/src/lib.rs`,
//! whose JSON is this module's: the connection's own messages are
//! [`PageConnectionMessage`] and [`BackendConnectionMessage`], an open names
//! a [`LaneTarget`], and a refusal is a [`LaneRefusal`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::ErrorBody;

/// The path of the page connection under `/api`.
pub const CONNECT_ROUTE: &str = "/connect";

/// How many of a lane's messages the page may send that the lane's owner has
/// not taken: the backend reads the connection on for the other lanes while
/// one owner handles a message, and holds this many for it, as a page sends
/// a few while one is handled, such as a steer behind an upload.
pub const LANE_CREDIT: u16 = 8;

/// The most lanes open on one connection, so that what a page's lanes hold
/// is bounded: a page keeps about twenty.
pub const MAX_LANES: usize = 64;

/// What a lane carries, as the page names it in the lane's open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LaneTarget {
    /// The synchronization channel.
    Sync,
    /// A conversation's lane: its frames, as the conversation's protocol has
    /// them.
    Conversation { conversation: String },
    /// A conversation's file watch.
    FileWatch { conversation: String },
    /// A device's folder dialog's file watch, through device access.
    DeviceWatch { device: String },
    /// A paired device's direct channel signaling.
    Signaling { device: String },
    /// A plugin's user stream of a conversation.
    UserStream { conversation: String, stream: String },
    /// What the core asks of the program that opened it
    /// (`crates/web-api-protocol/src/starter.rs`); only that program opens it.
    Starter,
    /// The core's requests of the program's plugin `plugin`
    /// (`crates/plugin-interface/src/wire.rs`); only the program opens it.
    Plugin { plugin: String },
}

/// Why the backend refused a lane's open: the status and the error answer a
/// request for it would have had.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LaneRefusal {
    pub status: u16,
    pub error: ErrorBody,
}

/// What the page says about the connection itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PageConnectionMessage {
    /// Asks the backend to answer `pong` with `id` at once, as the page does
    /// when it returns, to learn whether the connection is still alive.
    Ping { id: u32 },
}

/// What the backend says about the connection itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackendConnectionMessage {
    /// The answer to the page's ping `id`, ahead of any lane's message.
    Pong { id: u32 },
    /// Nothing else was sent for 30 seconds: the connection is quiet, not
    /// dead.
    Heartbeat,
}
