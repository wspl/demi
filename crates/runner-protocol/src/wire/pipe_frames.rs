//! How bytes move between the runner and the backend: pipes, and the frames of
//! a pipe's byte stream (data, idle, end).
//!
//! The same framing is a pipe's on every path, the relay's HTTP as much as a
//! direct channel, so a pipe moves between them without a second definition.
//!
//! # Pipes from the runner
//!
//! A pipe from the runner is one of two kinds:
//!
//! - A content pipe: one finite thing (a file, kept output, edit copies,
//!   media), sent as the body of `PUT /api/pipes/:id`.
//! - A stream pipe: bytes that must arrive as they are written (a resident
//!   stream's output, a job's input and output passed on while it runs), sent
//!   over a WebSocket at the same path.
//!
//! The difference is a TLS edge in front of the backend such as Cloudflare's.
//! It holds a request's body until it ends or passes about 2 MB, but passes
//! WebSocket messages as they come. The live browser view of a still page,
//! whose frames are small, showed nothing for minutes while its pictures waited
//! in a body.
//!
//! # Pipes the runner reads
//!
//! A pipe the runner reads is a `GET` whose response streams. The same edge
//! ends a response that sends nothing for about 100 seconds, its head included:
//! a command waiting silently, or a stop waiting for an absent runner, broke
//! their pipes with a 524. So the backend answers at once and frames the body.
//!
//! - Only the end frame ends the pipe. A body that stops without one is a
//!   broken pipe, never an ordinary end.
//! - Idle frames keep the response alive and never reach the reader.
//! - A stream pipe's WebSocket pings after the same idle time.
//!
//! # A pipe that resumes
//!
//! A call's standard output goes over a WebSocket of its own, whose messages
//! are the frames of a session (`crates/runner-protocol/src/wire/session.rs`):
//! each carries one data or end frame, and a connection that replaces another
//! opens with the count of messages the runner received, which the backend
//! writes again from. A drop of the connection ends nothing: the runner
//! connects again and the pipe goes on from where it got to. The runner closes
//! the WebSocket itself once it reads no more, at the pipe's end or when its
//! reader let go; a connection that closed without that was lost.
//!
//! The runner's HTTP client speaks HTTP/2 where the edge offers it, so a
//! concurrent pipe costs no new connection, which through a proxy and a TLS
//! edge costs two or three round trips.
//!
//! # A pipe between two devices
//!
//! A pipe whose source and sink are two runners that may talk directly moves
//! its bytes to the pair's direct channel and back
//! (`crates/runner-process/src/pipes/moves.rs`). Its relay legs stay for as
//! long as it lasts and carry its start, its end, its failure and its moves:
//!
//! - The backend tells the source its sink's device in a [`PipeSink`] text
//!   message on the source's WebSocket.
//! - The source says each move in a [`PipeMove`] text message, after the last
//!   byte it sent the old way.
//! - The sink reads each move as a [`PipeFrame::Moved`] frame, which names the
//!   source's device from the backend's record of the pipe, in order with the
//!   data frames.
//!
//! A move counts bytes from the pipe's first: the bytes from `at` on come the
//! new way. A move back to the relay may name a byte the sink holds already,
//! since the source sends again what the sink did not confirm; the sink drops
//! what it holds.

use std::time::Duration;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use serde::{Deserialize, Serialize};

/// How long a pipe's writer goes without a frame before it sends an idle
/// one, and a stream pipe's WebSocket without a message before a ping.
pub const PIPE_IDLE: Duration = Duration::from_secs(30);
/// The most bytes of one data frame; a writer splits larger chunks, so a
/// reader holds one frame at a time.
pub const PIPE_DATA_BYTES: usize = 64 * 1024;
/// The most bytes of an end frame's map, its cause's words included: a reader
/// holds the frame whole.
const PIPE_END_BYTES: usize = 64 * 1024;

/// The route of a call's standard output, which the runner reads over a
/// WebSocket that resumes (`/api/pipes/{id}` serves every other pipe).
pub const CALL_OUTPUT_ROUTE: &str = "/api/call-outputs";

/// The most bytes of one message of a pipe's session: a data or an end frame
/// and the session's head.
pub const PIPE_SESSION_MESSAGE_BYTES: usize = PIPE_DATA_BYTES + 64;

const DATA: u8 = 0x01;
const IDLE: u8 = 0x02;
const END: u8 = 0x03;
const MOVED: u8 = 0x04;
/// The most bytes of a moved frame's map: a device id and a position.
const PIPE_MOVED_BYTES: usize = 1024;
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
    /// The pipe's bytes from `at` on come over `to`; `source` is the device
    /// of the pipe's source, whose direct channel a move there names.
    Moved { to: PipePath, at: u64, source: String },
}

/// The way a pipe between two devices carries its bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipePath {
    /// Through the backend, as every pipe starts.
    Relay,
    /// The direct channel between the two runners.
    Direct,
}

/// A stream pipe's source's move, in a text message on its WebSocket: its
/// bytes from `at` on go over `to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipeMove {
    pub to: PipePath,
    pub at: u64,
}

/// What the backend tells a stream pipe's source in a text message on its
/// WebSocket: the device its sink is, once the sink is a runner the source's
/// may talk directly with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipeSink {
    pub sink: String,
}

/// A moved frame's map.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MovedRecord {
    to: PipePath,
    at: u64,
    source: String,
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
    #[error("an invalid pipe moved frame: {0}")]
    InvalidMoved(String),
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

/// The moved frame of a move to `to` at byte `at` of `source`'s pipe.
pub fn moved_frame(to: PipePath, at: u64, source: &str) -> Bytes {
    let record = MovedRecord {
        to,
        at,
        source: source.to_owned(),
    };
    let map = rmp_serde::to_vec_named(&record).expect("a moved record encodes");
    let mut frame = BytesMut::with_capacity(HEAD_BYTES + map.len());
    frame.put_u8(MOVED);
    frame.put_u32(map.len() as u32);
    frame.put_slice(&map);
    frame.freeze()
}

/// The frame a message of a pipe's session carries: one whole frame.
pub fn decode_message(message: &[u8]) -> Result<PipeFrame, PipeFrameError> {
    let mut decoder = PipeFrameDecoder::new();
    decoder.extend(message);
    match decoder.next_frame()? {
        Some(frame) if !decoder.is_partial() => Ok(frame),
        _ => Err(PipeFrameError::InvalidEnd(
            "a session message is not one whole frame".into(),
        )),
    }
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
            MOVED => PIPE_MOVED_BYTES,
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
        if kind == MOVED {
            let MovedRecord { to, at, source } = rmp_serde::from_slice(&body)
                .map_err(|error| PipeFrameError::InvalidMoved(error.to_string()))?;
            return Ok(Some(PipeFrame::Moved { to, at, source }));
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
