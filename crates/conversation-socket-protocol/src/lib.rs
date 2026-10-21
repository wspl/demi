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
//! # Nodes and subscriptions
//!
//! Every agent of a conversation, the root, a subagent or a side question, is
//! a node of its tree, named by its id; the root's is the conversation's. A
//! page subscribes to each node it shows and ends the subscription when it
//! stops showing it, and only subscribed nodes' messages reach it
//! (`docs/decisions/0009-agent-sessions-and-sync.md`).
//!
//! ```text
//! client                               server
//! subscribe { node, held } ----------> (kept until open)
//! open { agents, draft } ------------> attach to the live tree
//!                         <----------- opened
//!                         <----------- agents { version, since? }
//!                         <----------- request_output
//!                         <----------- transcript_reset { node }, each
//!                                      subscribed live node held stale
//!                         <----------- cache_chart { node }, each
//!                                      subscribed agent
//!                         <----------- shell_output, each live command
//!                         <----------- transcript_patch { node, revision }
//!                         <----------- agent { revision }
//! ```
//!
//! The handshake is one step: nothing happens to the tree between `opened`
//! and its last frame, so the snapshot frames agree with each other.
//!
//! # Versions
//!
//! Every collection carries a [`Version`]: a node's transcript, the agents of
//! a conversation. A node's cache chart needs none: it travels whole each
//! time, and the latest frame is the current chart.
//!
//! - A change carries the revision one past the previous one of its
//!   collection. A client applies a change whose revision is one past its own,
//!   ignores one whose revision is not greater, and subscribes again, naming
//!   what it holds, when it sees a gap.
//! - What the page holds stays on screen as the reset arrives; a reset from
//!   the blocks the page names replaces only what follows them.
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
mod ping;
mod server;
mod transcript;

pub use client::{
    ClientContent, ClientFrame, ClientFrameKind, EditRequest, MediaRef, RegenerateRequest, SavedDelivery,
};
pub use agents::{AgentEntry, AgentRole, HeldEdge, SideQuestion, Subagent};
pub use cost::{AgentCost, CacheChart, CacheMiss, ChartPoint, CostPrice, CostTotals};
pub use ping::PagePing;
pub use server::{
    AbortResult, AbortTarget, AgentCode, AttachmentSourceDto, CommandView, ConversationAttachment, EditOutcome,
    Failures, JobModel, JobPhase, Refusal,
    RequestOutput, ServerFrame, RemoveOutcome, ShellStatus, SteerOutcome, TurnSpeed,
};
pub use transcript::{
    BlockVersions, LIGHT_INPUT_BYTES, PAGE_BLOCKS, PageAt, TranscriptPatch, Version, Versions,
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
