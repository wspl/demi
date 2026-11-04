//! The frames of a WebSocket that carries lanes: a page's connection to the
//! backend (`crates/web-api-protocol/src/connection.rs`) and a runner's pipe
//! connection (`crates/runner-protocol/src/wire/pipe_connection.rs`).
//!
//! A lane carries what a WebSocket of its own would: text and binary
//! messages in order, and a close with a code and a reason. The connection
//! only numbers, cuts and paces them. Each connection says what its lanes'
//! opens name, what a refusal holds and how much credit a lane starts with.
//!
//! Every message of the connection is one binary WebSocket message, a frame:
//!
//! ```text
//! byte 0  kind
//!   0  connection   JSON: the connection's own message
//!   1  open         u32 lane, JSON: what the lane carries     client -> backend
//!   2  opened       u32 lane                                  backend -> client
//!   3  refused      u32 lane, JSON: why                       backend -> client
//!   4  data         u32 lane, u8 flags, a piece of a message  both ways
//!                   flags: bit 0 binary, bit 1 more pieces follow
//!   5  close        u32 lane, u16 code, UTF-8 reason          both ways
//!   6  credit       u32 lane, u16 messages                    backend -> client
//! ```
//!
//! Numbers are big-endian. The client numbers its lanes from 1 in each
//! connection and never uses a number twice in it, so a message of a lane
//! that has closed is dropped instead of reaching a lane opened after it.
//!
//! A lane's messages may follow its `open` at once, as a stream's data
//! follows its headers in HTTP/2: the backend holds them, within the lane's
//! credit, until it answers the open, and drops them if it refuses it. A
//! lane starts with its connection's credit of messages, and the backend
//! grants one more for each message the lane's owner took.
//!
//! The crate knows no page, runner or pipe.

use bytes::{BufMut as _, Bytes, BytesMut};

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

/// One frame of a lane connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// The connection's own message, as JSON.
    Connection(Bytes),
    /// `lane` opens to carry what `target`, its JSON, names.
    Open { lane: u32, target: Bytes },
    Opened { lane: u32 },
    /// The backend refused `lane`'s open, for what `refusal`, its JSON, says.
    Refused { lane: u32, refusal: Bytes },
    /// A piece of a message of `lane`'s.
    Data { lane: u32, flags: u8, piece: Bytes },
    Close { lane: u32, code: u16, reason: String },
    /// `messages` more of `lane`'s messages may come.
    Credit { lane: u32, messages: u16 },
}

impl Frame {
    /// Reads one frame; an error says how it breaks the format.
    pub fn decode(bytes: Bytes) -> Result<Self, &'static str> {
        let Some(&kind) = bytes.first() else {
            return Err("empty message");
        };
        if kind == CONNECTION_FRAME {
            return Ok(Self::Connection(bytes.slice(1..)));
        }
        let lane = bytes.get(1..5).ok_or("message without a lane")?;
        let lane = u32::from_be_bytes([lane[0], lane[1], lane[2], lane[3]]);
        let rest = bytes.slice(5..);
        match kind {
            OPEN_FRAME => Ok(Self::Open { lane, target: rest }),
            OPENED_FRAME => Ok(Self::Opened { lane }),
            REFUSED_FRAME => Ok(Self::Refused { lane, refusal: rest }),
            DATA_FRAME => {
                let &flags = rest.first().ok_or("data without flags")?;
                Ok(Self::Data {
                    lane,
                    flags,
                    piece: rest.slice(1..),
                })
            }
            CLOSE_FRAME => {
                let code = rest.get(..2).ok_or("close without a code")?;
                Ok(Self::Close {
                    lane,
                    code: u16::from_be_bytes([code[0], code[1]]),
                    reason: String::from_utf8_lossy(&rest[2..]).into_owned(),
                })
            }
            CREDIT_FRAME => {
                let messages = rest.get(..2).ok_or("credit without a count")?;
                Ok(Self::Credit {
                    lane,
                    messages: u16::from_be_bytes([messages[0], messages[1]]),
                })
            }
            _ => Err("unknown message kind"),
        }
    }

    /// The frame as one WebSocket message.
    pub fn encode(&self) -> Bytes {
        let (kind, lane, body): (u8, Option<u32>, &[u8]) = match self {
            Self::Connection(json) => (CONNECTION_FRAME, None, json),
            Self::Open { lane, target } => (OPEN_FRAME, Some(*lane), target),
            Self::Opened { lane } => (OPENED_FRAME, Some(*lane), &[]),
            Self::Refused { lane, refusal } => (REFUSED_FRAME, Some(*lane), refusal),
            Self::Data { lane, flags, piece } => {
                let mut frame = BytesMut::with_capacity(6 + piece.len());
                frame.put_u8(DATA_FRAME);
                frame.put_u32(*lane);
                frame.put_u8(*flags);
                frame.put_slice(piece);
                return frame.freeze();
            }
            Self::Close { lane, code, reason } => {
                let mut frame = BytesMut::with_capacity(7 + reason.len());
                frame.put_u8(CLOSE_FRAME);
                frame.put_u32(*lane);
                frame.put_u16(*code);
                frame.put_slice(reason.as_bytes());
                return frame.freeze();
            }
            Self::Credit { lane, messages } => {
                let mut frame = BytesMut::with_capacity(7);
                frame.put_u8(CREDIT_FRAME);
                frame.put_u32(*lane);
                frame.put_u16(*messages);
                return frame.freeze();
            }
        };
        let mut frame = BytesMut::with_capacity(5 + body.len());
        frame.put_u8(kind);
        if let Some(lane) = lane {
            frame.put_u32(lane);
        }
        frame.put_slice(body);
        frame.freeze()
    }
}

/// The data frames of `lane`'s message `message`, in pieces of at most
/// [`LANE_PIECE_BYTES`]; an empty message is one empty piece.
pub fn pieces(lane: u32, binary: bool, message: Bytes) -> impl Iterator<Item = Frame> {
    let len = message.len();
    let starts = (0..len.max(1)).step_by(LANE_PIECE_BYTES);
    starts.map(move |start| {
        let end = (start + LANE_PIECE_BYTES).min(len);
        let mut flags = 0;
        if binary {
            flags |= BINARY_FLAG;
        }
        if end < len {
            flags |= MORE_FLAG;
        }
        Frame::Data {
            lane,
            flags,
            piece: message.slice(start..end),
        }
    })
}

/// Puts a lane's message together from its pieces.
#[derive(Debug, Default)]
pub struct Assembly {
    pieces: Option<(bool, BytesMut)>,
}

/// A whole message of a lane's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Text(String),
    Binary(Bytes),
}

/// Why a lane's message could not be put together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssemblyError {
    /// Longer than the lane takes.
    TooBig,
    /// A text message that is not UTF-8.
    NotUtf8,
}

impl Assembly {
    /// Takes a data piece with `flags`: the message it ends, if any, of at
    /// most `max` bytes.
    pub fn take(&mut self, flags: u8, piece: Bytes, max: usize) -> Result<Option<Message>, AssemblyError> {
        let more = flags & MORE_FLAG != 0;
        let (binary, whole) = match self.pieces.take() {
            None if !more => (flags & BINARY_FLAG != 0, piece),
            pieces => {
                let (binary, mut whole) = pieces.unwrap_or_else(|| (flags & BINARY_FLAG != 0, BytesMut::new()));
                if whole.len() + piece.len() > max {
                    return Err(AssemblyError::TooBig);
                }
                whole.put_slice(&piece);
                if more {
                    self.pieces = Some((binary, whole));
                    return Ok(None);
                }
                (binary, whole.freeze())
            }
        };
        if whole.len() > max {
            return Err(AssemblyError::TooBig);
        }
        if binary {
            return Ok(Some(Message::Binary(whole)));
        }
        String::from_utf8(whole.into())
            .map(|text| Some(Message::Text(text)))
            .map_err(|_| AssemblyError::NotUtf8)
    }

    /// Drops the pieces of a message still coming.
    pub fn clear(&mut self) {
        self.pieces = None;
    }
}
