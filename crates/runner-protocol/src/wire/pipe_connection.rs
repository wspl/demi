//! The pipe connection: one more WebSocket a runner holds to the backend,
//! `WS /api/runner/pipes`, beside its link, whose lanes carry the stream
//! pipes the runner writes (`crates/runner-process/src/pipes/lanes.rs`).
//!
//! Its frames are `crates/shared-lanes/src/lib.rs`'s. An open names a
//! [`PipeLaneTarget`] and a refusal is a [`PipeLaneRefusal`]; the connection
//! has no message of its own, and WebSocket pings keep it through an edge
//! that ends a silent connection. A lane carries what the pipe's own
//! WebSocket would (`pipe_frames`): the pipe's bytes in binary messages of
//! at most [`STREAM_PIPE_MESSAGE_BYTES`](super::STREAM_PIPE_MESSAGE_BYTES),
//! the backend's [`PipeSink`](super::PipeSink) and the runner's
//! [`PipeMove`](super::PipeMove) in text messages, and the pipe's end in a
//! close with the code and reason the WebSocket's close frame would carry.
//!
//! The link carries no pipe's bytes: it keeps each message until the backend
//! confirms it, which would make a second copy of a stream, and its control
//! would wait behind them.

use serde::{Deserialize, Serialize};

/// The path of the pipe connection.
pub const PIPE_CONNECTION_ROUTE: &str = "/api/runner/pipes";

/// How many of a lane's messages the runner may send that the backend's pipe
/// has not taken: at most [`STREAM_PIPE_MESSAGE_BYTES`](super::STREAM_PIPE_MESSAGE_BYTES)
/// each, 1 MiB, what the relay keeps in flight for one pipe at 2 MB/s over
/// a half-second round trip. Without a bound, a pipe whose sink reads slowly
/// would hold its whole source in the backend's memory.
pub const PIPE_LANE_CREDIT: u16 = 16;

/// What a lane of the pipe connection carries, as the runner names it in the
/// lane's open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PipeLaneTarget {
    /// The stream pipe `pipe` this runner writes, its source.
    StreamPipe { pipe: String },
}

/// Why the backend refused a lane's open: the status and the words of the
/// answer a request for the pipe would have had.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipeLaneRefusal {
    pub status: u16,
    pub message: String,
}
