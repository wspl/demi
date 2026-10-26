//! The page connection: the one WebSocket a page holds to the backend,
//! `WS /api/connect?page=<page id>`, and the lanes it carries.
//!
//! A lane carries what one of the page's parts says with the backend: the
//! synchronization channel, a conversation's lane, a file watch, a device's
//! signaling or a user stream. Each lane's messages are its protocol's, as
//! they would be on a WebSocket of its own; the connection only numbers,
//! cuts and paces them.
//!
//! Every message of the connection is one binary WebSocket message, a frame:
//!
//! ```text
//! byte 0  kind
//!   0  connection   JSON: PageConnectionMessage or BackendConnectionMessage
//!   1  open         u32 lane, JSON LaneTarget                  page -> backend
//!   2  opened       u32 lane                                   backend -> page
//!   3  refused      u32 lane, JSON LaneRefusal                 backend -> page
//!   4  data         u32 lane, u8 flags, a piece of a message   both ways
//!                   flags: bit 0 binary, bit 1 more pieces follow
//!   5  close        u32 lane, u16 code, UTF-8 reason           both ways
//!   6  credit       u32 lane, u16 messages                     backend -> page
//! ```
//!
//! Numbers are big-endian. The page numbers its lanes from 1 in each
//! connection and never uses a number twice in it, so a message of a lane
//! that has closed is dropped instead of reaching a lane opened after it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::ErrorBody;

/// The path of the page connection under `/api`.
pub const CONNECT_ROUTE: &str = "/connect";

/// A frame's kind: JSON of the connection's own message.
pub const CONNECTION_FRAME: u8 = 0;
/// A frame's kind: a lane's open.
pub const OPEN_FRAME: u8 = 1;
/// A frame's kind: the backend's word that a lane opened.
pub const OPENED_FRAME: u8 = 2;
/// A frame's kind: the backend's refusal of a lane's open.
pub const REFUSED_FRAME: u8 = 3;
/// A frame's kind: a piece of a lane's message.
pub const DATA_FRAME: u8 = 4;
/// A frame's kind: a lane's close.
pub const CLOSE_FRAME: u8 = 5;
/// A frame's kind: the backend's credit for a lane's next messages.
pub const CREDIT_FRAME: u8 = 6;

/// A data piece's flag: the message is binary, not UTF-8 text.
pub const BINARY_FLAG: u8 = 1;
/// A data piece's flag: more pieces of the message follow.
pub const MORE_FLAG: u8 = 2;

/// The most bytes of a message a data piece carries, so that a large message
/// of one lane holds another's back by no more than one piece.
pub const LANE_PIECE_BYTES: usize = 16 * 1024;

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
