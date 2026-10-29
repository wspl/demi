//! The conversation WebSocket's frames: what the web app sends
//! ([`ClientFrame`]) and what the backend answers ([`ServerFrame`]), with
//! transcript patches and versions.
//!
//! The crate holds types and their checks; the session logic and the socket
//! live elsewhere, and no frame carries file bytes.
//!
//! Client frames are strict: an unknown field is refused. Server frames are
//! tolerant, so a page accepts fields it does not know.
//!
//! # Nodes and what a socket follows
//!
//! Every agent of a conversation, the root, a subagent or a side question, is
//! a node of its tree, named by its id; the root's is the conversation's.
//!
//! The socket carries changes and answers no read.
//! A read never needs the socket: the page reads what it shows by the web API's
//! routes, draws it, and opens the socket after the reads it follows have
//! answered. Its `open` names each value it holds and follows with the version
//! it holds ([`Follow`]); a value it reads later, such as a subagent chosen
//! afterwards, it follows by a `follow` frame of its own, and `unfollow` ends a
//! follow:
//!
//! - the conversation's metadata, its saved messages, its workflows and its
//!   shares are whole values, which carry an entity tag;
//! - the conversation's agents and each node's transcript carry a [`Version`].
//!
//! The server registers the follow, then compares the value's current version
//! with the one held, where the value changes, so no change falls between the
//! two: the same version sends nothing, another whole value is sent as it is,
//! and a transcript past the version held is sent the patches after it from
//! the recent patches of its running node. A version no recent change reaches
//! is answered `stale`: the follow ends, and the page reads the value again
//! and follows it again. A page that holds a version of the store, read
//! before a tree was restored from it, is answered `rebased` when the tree
//! holds what it read: it moves its copy to the live version without a read.
//!
//! ```text
//! client                               server
//! GET .. (reads, no socket) --------->
//!                         <----------- the reads' answers
//! open { follows } ------------------> attach, follow, compare
//!                         <----------- opened
//!                         <----------- what changed since the versions held,
//!                                      or stale { value }
//!                         <----------- transcript_patch { node, version }
//!                         <----------- agent { version }
//!                         <----------- request_output, cache_chart,
//!                                      shell_output, as they change
//! ```
//!
//! # Versions
//!
//! A node's transcript and the agents of a conversation carry a [`Version`],
//! which their logs own; no other value does. A page reads each by a route,
//! which answers the version it read at, and follows it at that version.
//!
//! - A change carries the version after it, whose revision is one past the
//!   previous one of its collection. A page applies a change whose revision
//!   is one past what it holds, ignores one whose revision is not greater,
//!   and, on a gap or another epoch, unfollows the value, reads it again and
//!   follows it again.
//! - A collection no live session runs is at the epoch [`STORED_EPOCH`], at a
//!   revision derived from what the store holds of it, so it changes with
//!   every save that changes what a page shows.
//! - A response that asks for no tool holds the frames of its turn's end until
//!   the turn goes on or the conversation goes idle after its closing save, so
//!   the last patch and the root going idle go together. The hold waits for
//!   that save, never for a timer.
//!
//! A transcript travels in its light form (`light`): what only an open row
//! shows, a tool's output or a model's thinking, is read when the row opens.
//! A page of it is bounded by its size, wherever requests begin and end
//! ([`page`]), and every block names the request it belongs to, so any
//! window finds each edit's request.
//!
//! The agent resolves the files a client frame's content refers to through the
//! backend (`agent-server`'s `ContentResolver`) before the session sees the
//! content.

mod agents;
mod client;
mod cost;
mod follow;
mod server;
mod transcript;

pub use client::{
    ClientContent, ClientFrame, ClientFrameKind, EditRequest, MediaRef, RegenerateRequest, SavedDelivery, SendMessage,
};
pub use agents::{AgentEntry, AgentRole, SideQuestion, Subagent};
pub use cost::{AgentCost, CacheChart, CacheMiss, ChartPoint, CostPrice, CostTotals};
pub use follow::{Follow, Followed, Held};
pub use server::{
    AbortResult, AbortTarget, AgentCode, AttachmentSourceDto, CommandView, ConversationAttachment, EditOutcome, ReplyBlockFacts,
    Failures, JobModel, JobPhase, Refusal,
    RequestOutput, ServerFrame, RemoveOutcome, ShellStatus, SteerOutcome, TurnSpeed,
};
pub use transcript::{
    BlockVersions, FIRST_BLOCKS, LIGHT_INPUT_BYTES, PAGE_BLOCKS, PageAt, STORED_EPOCH, TranscriptPatch, Version, Versions,
    index_u32, latest_page_start, light, page,
};

use demi_shared_types::DecodeError;

/// Why a client frame was refused.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// The message is not JSON; the connection closes.
    #[error("the message is not JSON: {0}")]
    NotJson(serde_json::Error),
    /// The message is JSON but not a valid frame; the backend answers an
    /// `error` frame with the code `invalid_frame` and the connection stays
    /// open.
    #[error("invalid frame: {0}")]
    Invalid(DecodeError),
}

/// Decodes one message of the conversation WebSocket into a frame and checks
/// it, before anything acts on it.
pub fn decode_client_frame(text: &str) -> Result<ClientFrame, FrameError> {
    let frame: ClientFrame = match demi_shared_types::decode(text) {
        Ok(frame) => frame,
        Err(DecodeError::Syntax(error)) => return Err(FrameError::NotJson(error)),
        Err(error) => return Err(FrameError::Invalid(error)),
    };
    frame
        .check_content()
        .map_err(|report| FrameError::Invalid(DecodeError::from(report)))?;
    Ok(frame)
}
