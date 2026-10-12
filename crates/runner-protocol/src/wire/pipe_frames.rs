//! The frames of a pipe's byte stream the runner reads (`runner.md` § Pipes
//! and output): data, idle and end. An edge such as Cloudflare's ends a
//! response silent for about 100 seconds, so the writer sends an idle frame
//! after [`PIPE_IDLE`] without a frame, and only an end frame ends the pipe:
//! a body that stops without one is a broken pipe. Every transport between
//! the runner and the backend carries a pipe in these frames.

use std::time::Duration;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use serde::{Deserialize, Serialize};

/// How long a pipe's writer goes without a frame before it sends an idle
/// one, and a stream pipe's WebSocket without a message before a ping.
pub const PIPE_IDLE: Duration = Duration::from_secs(30);
/// The most bytes of one data frame; a writer splits larger chunks.
pub const PIPE_DATA_BYTES: usize = 64 * 1024;
/// The most bytes of an end frame's map, its cause's words included.
const PIPE_END_BYTES: usize = 64 * 1024;

const DATA: u8 = 0x01;
const IDLE: u8 = 0x02;
const END: u8 = 0x03;
/// A frame's type byte and its length.
const HEAD_BYTES: usize = 5;

/// How a pipe ended: whole, or broken with the words its reader reports.
pub type PipeEnd = Result<(), String>;

/// One frame of a pipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipeFrame {
    /// The pipe's next bytes, at most [`PIPE_DATA_BYTES`].
    Data(Bytes),
    /// Nothing to send yet.
    Idle,
    /// The pipe's end.
    End(PipeEnd),
}

/// An end frame's map: `{ ok: true }` or `{ ok: false, cause }`.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EndRecord {
    ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cause: Option<String>,
}

/// Why a pipe's bytes are not frames.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PipeFrameError {
    #[error("unknown pipe frame type {0:#04x}")]
    UnknownType(u8),
    #[error("a pipe frame of {length} bytes, over the {limit}-byte limit")]
    TooLong { length: usize, limit: usize },
    #[error("an invalid pipe end frame: {0}")]
    InvalidEnd(String),
}

/// Appends `bytes` to `frames` as data frames of at most
/// [`PIPE_DATA_BYTES`] each; empty bytes make no frame.
pub fn encode_data(bytes: &[u8], frames: &mut BytesMut) {
    for part in bytes.chunks(PIPE_DATA_BYTES) {
        frames.reserve(HEAD_BYTES + part.len());
        frames.put_u8(DATA);
        frames.put_u32(part.len() as u32);
        frames.put_slice(part);
    }
}

/// The idle frame.
pub fn idle_frame() -> Bytes {
    Bytes::from_static(&[IDLE])
}

/// The end frame for `end`.
pub fn end_frame(end: &PipeEnd) -> Bytes {
    let record = match end {
        Ok(()) => EndRecord {
            ok: true,
            cause: None,
        },
        Err(cause) => EndRecord {
            ok: false,
            cause: Some(cause.clone()),
        },
    };
    let map = rmp_serde::to_vec_named(&record).expect("an end record encodes");
    let mut frame = BytesMut::with_capacity(HEAD_BYTES + map.len());
    frame.put_u8(END);
    frame.put_u32(map.len() as u32);
    frame.put_slice(&map);
    frame.freeze()
}

/// Reads frames from a pipe's bytes as they arrive, in whatever pieces.
#[derive(Default)]
pub struct PipeFrameDecoder {
    buffer: BytesMut,
}

impl PipeFrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the next bytes of the stream.
    pub fn extend(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// The next whole frame, or none until more bytes arrive.
    pub fn next_frame(&mut self) -> Result<Option<PipeFrame>, PipeFrameError> {
        let Some(&kind) = self.buffer.first() else {
            return Ok(None);
        };
        let limit = match kind {
            IDLE => {
                self.buffer.advance(1);
                return Ok(Some(PipeFrame::Idle));
            }
            DATA => PIPE_DATA_BYTES,
            END => PIPE_END_BYTES,
            kind => return Err(PipeFrameError::UnknownType(kind)),
        };
        if self.buffer.len() < HEAD_BYTES {
            return Ok(None);
        }
        let length = u32::from_be_bytes(self.buffer[1..HEAD_BYTES].try_into().expect("four bytes"))
            as usize;
        if length > limit {
            return Err(PipeFrameError::TooLong { length, limit });
        }
        if self.buffer.len() < HEAD_BYTES + length {
            return Ok(None);
        }
        self.buffer.advance(HEAD_BYTES);
        let body = self.buffer.split_to(length).freeze();
        if kind == DATA {
            return Ok(Some(PipeFrame::Data(body)));
        }
        let record: EndRecord = rmp_serde::from_slice(&body)
            .map_err(|error| PipeFrameError::InvalidEnd(error.to_string()))?;
        let end = match record {
            EndRecord {
                ok: true,
                cause: None,
            } => Ok(()),
            EndRecord {
                ok: false,
                cause: Some(cause),
            } => Err(cause),
            EndRecord { ok, .. } => {
                let reason = if ok {
                    "a whole pipe's end names a cause"
                } else {
                    "a broken pipe's end names no cause"
                };
                return Err(PipeFrameError::InvalidEnd(reason.into()));
            }
        };
        Ok(Some(PipeFrame::End(end)))
    }

    /// Whether part of a frame is still waiting for its bytes.
    pub fn is_partial(&self) -> bool {
        !self.buffer.is_empty()
    }
}
