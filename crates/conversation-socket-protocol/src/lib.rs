//! The conversation WebSocket's frames: what the web app sends ([`ClientFrame`])
//! and what the backend answers ([`ServerFrame`]), with transcript patches
//! and versions. It holds types and their checks; the session logic and the
//! socket live elsewhere, and no frame carries file bytes.
//!
//! Client frames are strict: an unknown field is refused. Server frames are
//! tolerant, so a page accepts fields it does not know.
//!
//! The open handshake, for a page that holds blocks up to `from` and asks the
//! stream for those from there on:
//!
//! ```text
//! client                          server
//! open { from, edge } ----------> attach to the live tree
//!                    <----------- opened
//!                    <----------- transcript_reset { start, version }
//!                    <----------- phase, queue, pending_steers,
//!                                 pending_calls, waiting
//!                    <----------- each live subagent, depth first
//!                    <----------- shell_output, each live command
//!                    <----------- transcript_patch { revision + 1 }
//! ```
//!
//! The handshake is one step: nothing happens to the session between `opened`
//! and its last frame, so the snapshot frames agree with each other. Each
//! `transcript_patch` carries the revision one past the previous frame's: a
//! client applies a patch whose revision is one past its own, ignores one
//! whose revision is not greater, and asks for a fresh transcript with
//! `sync_transcript` when it sees a gap. A response that asks for no tool
//! holds the frames of its turn's end until the turn goes on or the
//! conversation goes idle after its closing save, so the last patch and
//! `phase: idle` go together; the hold waits for that save, never for a timer.
//!
//! The agent resolves the files a client frame's content refers to through the
//! backend (`agent-server`'s `ContentResolver`) before the session sees the
//! content.

mod client;
mod cost;
mod ping;
mod server;
mod transcript;

pub use client::{
    ClientContent, ClientFrame, ClientFrameKind, EditRequest, MediaRef, RegenerateRequest, SavedDelivery,
};
pub use cost::{AgentCost, CacheMiss, CostCall, CostPrice, CostTotals};
pub use ping::PagePing;
pub use server::{
    AbortResult, AbortTarget, AgentCode, CommandView, EditOutcome, Failures, JobModel, JobPhase, Refusal,
    ServerFrame, RemoveOutcome, ShellStatus, SteerOutcome, SubagentChild, SubagentEvent, SubagentJob, SubagentSummary,
};
pub use transcript::{
    BlockVersions, LIGHT_INPUT_BYTES, PAGE_BYTES, PageAt, TranscriptPatch, TranscriptVersion, Versions,
    index_u32, latest_page_start, latest_page_start_within, light,
    least_light_bytes, light_bytes, page, requests, reset_start,
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
