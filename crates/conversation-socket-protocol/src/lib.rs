//! The conversation WebSocket's frames (`crates-and-packages.md`
//! § conversation-socket-protocol; behavior in `runtime.md` § Frame protocol): what the
//! web app sends ([`ClientFrame`]) and what the backend answers
//! ([`ServerFrame`]), with transcript patches and versions. It holds types
//! and their checks; the session logic and the socket live elsewhere, and no
//! frame carries file bytes.
//!
//! Client frames are strict: an unknown field is refused. Server frames are
//! tolerant, so a page accepts fields it does not know.

mod client;
mod server;
mod transcript;

pub use client::{ClientContent, ClientFrame, ClientFrameKind, EditRequest, MediaRef};
pub use server::{
    AbortResult, AbortTarget, CommandView, EditOutcome, Failures, JobPhase, ServerFrame,
    ShellStatus, SteerOutcome, SubagentEvent, SubagentJob,
};
pub use transcript::{
    LIGHT_INPUT_BYTES, PAGE_BYTES, PageAt, TranscriptPatch, TranscriptVersion, index_u32, latest_page_start, light,
    light_bytes, page, requests, reset_start,
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
