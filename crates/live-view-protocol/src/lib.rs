//! The live view's protocol where it is not about its source: the frames a user
//! stream carries, the pictures' header and codec, the viewer's input in the
//! picture's coordinates, and the messages that pace the pictures. A source's
//! own protocol, such as the browser's, carries these beside its own messages.
//! It holds types and their checks only; the Host's side of them is
//! `live-view`, the page's `@demicodes/web-ui`'s live module.
//!
//! - The codec is fixed, H.264 High level 5.1, and the stream does not name it:
//!   the page asks the user's browser for a decoder before it opens a view.
//!   Page and Host ship in the same release, so the protocol carries no
//!   version.
//! - Video chunks are at most 16 KiB so every 16 KiB the sender puts on the
//!   wire is acknowledged on its own.
//! - A frame the protocol refuses (bad length or kind, a header cut short, a
//!   control message that is not JSON or does not match its schema) is a defect
//!   of its sender and ends the view. Each view reads its own stream from the
//!   first byte, so a frame an ended stream left unfinished never reaches the
//!   next.

use bytes::{Buf, Bytes, BytesMut};
use demi_shared_types::{DecodeError, MAX_SAFE_INTEGER};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

#[cfg(feature = "testing")]
pub mod testing;

/// A frame's kind, the byte after its length: a JSON control message.
pub const CONTROL_FRAME: u8 = 1;
/// A chunk of a video frame, from the Host: [`VideoHeader`], then its part
/// of the frame's H.264 Annex B data.
pub const VIDEO_FRAME: u8 = 2;
/// The page's acknowledgements of video chunks: one or more [`ChunkAck`]s.
/// A source's own protocol numbers its other kinds from 4.
pub const ACK_FRAME: u8 = 3;
/// The most bytes a video chunk takes on the wire, its length and kind
/// included, so that every 16 KiB the sender puts on the path is
/// acknowledged on its own.
pub const CHUNK_BYTES: usize = 16 * 1024;
/// The video frames' codec as WebCodecs names it: H.264 High profile (`64`),
/// no constraint flags (`00`), level 5.1 (`33`). Every source encodes with
/// it, and the page asks the user's browser for a decoder of it before it
/// opens a view.
pub const VIDEO_CODEC: &str = "avc1.640033";
/// The most bits per second [`VIDEO_CODEC`]'s level carries: level 5.1's
/// maximum video bit rate (H.264 Table A-1). The browser's capture
/// extension's software encoder refuses to start above the level's limit
/// ("Failed to initialize OpenH264 encoder", measured with the pinned Chrome
/// between 288 and 299 Mbps), so a budget never asks for more.
pub const VIDEO_MAX_BITRATE: u32 = 240_000_000;
/// The largest frame after its length: a source's largest message.
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
/// How often the Host speaks at least, so a still picture is told from a stall.
pub const HEARTBEAT_MS: u64 = 250;
/// Silence after which the page shows the stream as stalled and stops sending input.
pub const STALL_MS: u64 = 1000;

/// One frame of a live view's stream: its kind and payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: u8,
    pub payload: Bytes,
}

/// A frame the protocol refuses.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("a live frame is empty or too large")]
    Length,
}

/// Splits a live view's bytes into its frames, a four-byte big-endian length,
/// a kind and the payload, as they arrive: the stream has no message
/// boundaries.
#[derive(Debug, Default)]
pub struct FrameSplitter {
    pending: BytesMut,
}

impl FrameSplitter {
    pub fn push(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
    }

    /// The next whole frame, the whole of it as it came, its length and kind
    /// included; none until more bytes arrive.
    pub fn next_raw(&mut self) -> Result<Option<Bytes>, FrameError> {
        let Some(length) = self.pending.first_chunk::<4>() else {
            return Ok(None);
        };
        let length = usize::try_from(u32::from_be_bytes(*length)).unwrap_or(usize::MAX);
        if length == 0 || length > MAX_FRAME_BYTES {
            return Err(FrameError::Length);
        }
        if self.pending.len() < 4 + length {
            return Ok(None);
        }
        Ok(Some(self.pending.split_to(4 + length).freeze()))
    }

    /// The next whole frame; none until more bytes arrive.
    pub fn next(&mut self) -> Result<Option<Frame>, FrameError> {
        Ok(self.next_raw()?.map(|mut raw| {
            raw.advance(4);
            let kind = raw.get_u8();
            Frame { kind, payload: raw }
        }))
    }

    /// Whether a frame was cut short: bytes wait that make no whole frame.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// The kind of a whole frame as [`FrameSplitter::next_raw`] gives it.
pub fn raw_kind(raw: &[u8]) -> u8 {
    raw[4]
}

/// What a picture's source is called in its view, such as a browser tab's
/// `t1`: ASCII without control characters, at most
/// [`VideoHeader::SOURCE_BYTES`] bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceId(String);

impl SourceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for SourceId {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let fits = !value.is_empty()
            && value.len() <= VideoHeader::SOURCE_BYTES
            && value.bytes().all(|byte| byte.is_ascii_graphic());
        if fits {
            Ok(Self(value.to_owned()))
        } else {
            Err(format!("{value:?} is not a picture's source"))
        }
    }
}

/// A video chunk's header, big-endian: the source's ID (ASCII, padded with
/// zero bytes to 16), the stream generation (u32), the frame's sequence
/// number (u32), the chunk's index and the frame's chunk count (u16 each),
/// flags (u8, 1 = key frame), three reserved bytes, the frame's timestamp in
/// microseconds (f64), and the picture's width and height in pixels (u16
/// each). Every chunk but a frame's last carries [`VideoHeader::CHUNK_DATA`]
/// bytes of its data, so a chunk's index places its data in the frame.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoHeader {
    pub source: SourceId,
    pub generation: u32,
    pub sequence: u32,
    pub chunk: u16,
    pub chunks: u16,
    pub key: bool,
    pub timestamp: f64,
    pub width: u16,
    pub height: u16,
}

impl VideoHeader {
    pub const BYTES: usize = 44;
    /// The source ID's field, which holds the longest browser tab ID, `t` and
    /// 15 digits.
    pub const SOURCE_BYTES: usize = 16;
    /// The frame data a chunk carries, but for a frame's last: what
    /// [`CHUNK_BYTES`] leaves after the length, the kind and the header.
    pub const CHUNK_DATA: usize = CHUNK_BYTES - 5 - Self::BYTES;

    /// Appends the header to `bytes`.
    pub fn write(&self, bytes: &mut Vec<u8>) {
        let source = self.source.as_str().as_bytes();
        bytes.extend_from_slice(source);
        bytes.resize(bytes.len() + Self::SOURCE_BYTES - source.len(), 0);
        bytes.extend_from_slice(&self.generation.to_be_bytes());
        bytes.extend_from_slice(&self.sequence.to_be_bytes());
        bytes.extend_from_slice(&self.chunk.to_be_bytes());
        bytes.extend_from_slice(&self.chunks.to_be_bytes());
        bytes.push(u8::from(self.key));
        bytes.extend_from_slice(&[0; 3]);
        bytes.extend_from_slice(&self.timestamp.to_be_bytes());
        bytes.extend_from_slice(&self.width.to_be_bytes());
        bytes.extend_from_slice(&self.height.to_be_bytes());
    }

    /// Splits a video chunk's payload into its header and data.
    pub fn split(payload: &[u8]) -> Result<(Self, &[u8]), DecodeError> {
        let Some((header, data)) = payload.split_first_chunk::<{ Self::BYTES }>() else {
            return Err(DecodeError::Invalid(
                "a video chunk is shorter than its header".into(),
            ));
        };
        let (source, fields) = header.split_at(Self::SOURCE_BYTES);
        let length = source
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(source.len());
        let source = std::str::from_utf8(&source[..length])
            .map_err(|error| DecodeError::Invalid(error.to_string()))
            .and_then(|source| SourceId::try_from(source).map_err(DecodeError::Invalid))?;
        let bytes = |at: usize, count: usize| &fields[at..at + count];
        let header = Self {
            source,
            generation: u32::from_be_bytes(bytes(0, 4).try_into().expect("four bytes")),
            sequence: u32::from_be_bytes(bytes(4, 4).try_into().expect("four bytes")),
            chunk: u16::from_be_bytes(bytes(8, 2).try_into().expect("two bytes")),
            chunks: u16::from_be_bytes(bytes(10, 2).try_into().expect("two bytes")),
            key: fields[12] & 1 == 1,
            timestamp: f64::from_be_bytes(bytes(16, 8).try_into().expect("eight bytes")),
            width: u16::from_be_bytes(bytes(24, 2).try_into().expect("two bytes")),
            height: u16::from_be_bytes(bytes(26, 2).try_into().expect("two bytes")),
        };
        if header.chunks == 0 || header.chunk >= header.chunks {
            return Err(DecodeError::Invalid(format!(
                "chunk {} of a frame of {} chunks",
                header.chunk, header.chunks
            )));
        }
        Ok((header, data))
    }
}

/// The page's acknowledgement of one video chunk, big-endian: the stream
/// generation and the frame's sequence number (u32 each), the chunk's index
/// (u16), two reserved bytes, and when the page received the chunk, in
/// milliseconds on a clock of the page's own (f64), whose differences alone
/// mean anything.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChunkAck {
    pub generation: u32,
    pub sequence: u32,
    pub chunk: u16,
    pub received: f64,
}

impl ChunkAck {
    pub const BYTES: usize = 20;

    /// Appends the acknowledgement to `bytes`.
    pub fn write(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.generation.to_be_bytes());
        bytes.extend_from_slice(&self.sequence.to_be_bytes());
        bytes.extend_from_slice(&self.chunk.to_be_bytes());
        bytes.extend_from_slice(&[0; 2]);
        bytes.extend_from_slice(&self.received.to_be_bytes());
    }

    /// The acknowledgements of an acknowledgement frame's payload.
    pub fn split(payload: &[u8]) -> Result<Vec<Self>, DecodeError> {
        if payload.is_empty() || payload.len() % Self::BYTES != 0 {
            return Err(DecodeError::Invalid(format!(
                "an acknowledgement frame of {} bytes",
                payload.len()
            )));
        }
        Ok(payload
            .chunks_exact(Self::BYTES)
            .map(|entry| {
                let bytes = |at: usize, count: usize| &entry[at..at + count];
                Self {
                    generation: u32::from_be_bytes(bytes(0, 4).try_into().expect("four bytes")),
                    sequence: u32::from_be_bytes(bytes(4, 4).try_into().expect("four bytes")),
                    chunk: u16::from_be_bytes(bytes(8, 2).try_into().expect("two bytes")),
                    received: f64::from_be_bytes(bytes(12, 8).try_into().expect("eight bytes")),
                }
            })
            .collect())
    }
}

/// The viewer's platform, which decides how its keys map on the Host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Mac,
    Windows,
    Linux,
    Other,
}

/// What pointing input a source takes, which the page sends it: a mouse's
/// pointer with its buttons, or touches with their contacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PointerKind {
    Mouse,
    Touch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PointerAction {
    Move,
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PointerButton {
    None,
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TouchAction {
    Start,
    Move,
    End,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum KeyAction {
    Down,
    Up,
}

/// One finger on the picture.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct TouchContact {
    /// The finger, as long as it touches: the page's pointer ID.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub id: u64,
    #[garde(range(min = 0.0, max = 4096.0))]
    pub x: f64,
    #[garde(range(min = 0.0, max = 4096.0))]
    pub y: f64,
}

/// The most fingers a touch carries.
pub const MAX_CONTACTS: usize = 10;

/// The viewer's input, in the picture's coordinates: the CSS pixels of a
/// browser tab, the points of a simulator's screen. A source turns it into
/// its own input; the Host delivers each viewer's input in the order it
/// arrives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum ViewerInput {
    #[serde(rename_all = "camelCase")]
    Pointer {
        #[garde(skip)]
        action: PointerAction,
        #[garde(range(min = 0.0, max = 4096.0))]
        x: f64,
        #[garde(range(min = 0.0, max = 4096.0))]
        y: f64,
        #[garde(skip)]
        button: PointerButton,
        #[garde(range(max = 31))]
        buttons: u8,
        #[garde(range(max = 3))]
        click_count: u8,
        /// The modifier keys held, as CDP numbers them: Alt 1, Control 2,
        /// Meta 4, Shift 8.
        #[garde(range(max = 15))]
        modifiers: u8,
    },
    /// Fingers on the picture: those down after a start or a move, the one
    /// lifted with an end, none with a cancel.
    Touch {
        #[garde(skip)]
        action: TouchAction,
        #[garde(length(max = MAX_CONTACTS), dive)]
        contacts: Vec<TouchContact>,
    },
    #[serde(rename_all = "camelCase")]
    Wheel {
        #[garde(range(min = 0.0, max = 4096.0))]
        x: f64,
        #[garde(range(min = 0.0, max = 4096.0))]
        y: f64,
        #[garde(range(min = -10_000.0, max = 10_000.0))]
        delta_x: f64,
        #[garde(range(min = -10_000.0, max = 10_000.0))]
        delta_y: f64,
        #[garde(range(max = 15))]
        modifiers: u8,
    },
    #[serde(rename_all = "camelCase")]
    Key {
        #[garde(skip)]
        action: KeyAction,
        #[garde(length(chars, max = 64))]
        key: String,
        #[garde(length(chars, max = 64))]
        code: String,
        #[garde(skip)]
        key_code: u8,
        #[garde(range(max = 15))]
        modifiers: u8,
        #[garde(skip)]
        repeat: bool,
        #[garde(range(max = 3))]
        location: u8,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "String")]
        #[garde(length(chars, min = 1, max = 16))]
        text: Option<String>,
        #[garde(skip)]
        alt_graph: bool,
    },
    /// Committed text: an input method's result.
    Text {
        #[garde(length(chars, max = 20_000))]
        text: String,
    },
    /// An input method's text being composed.
    Composition {
        #[garde(length(chars, max = 20_000))]
        text: String,
    },
}

/// The viewer's first message: its platform.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LiveHello {
    #[garde(skip)]
    pub platform: Platform,
}

/// The page's decoder could not decode a frame of `generation`; the next
/// frame must be a key frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LiveKeyframe {
    #[garde(skip)]
    pub generation: u32,
}

/// Release every key, button and finger this viewer holds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LiveRelease {}

/// Sent whenever the Host has had nothing else to say for [`HEARTBEAT_MS`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LiveHeartbeat {}

/// Something the viewer asked for failed; the stream goes on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LiveNotice {
    /// A code of the source's set, which the page has words for.
    #[garde(skip)]
    pub code: String,
    #[garde(skip)]
    pub message: String,
}

/// A source's protocol of messages to the page, which carries the live
/// view's own: the Host's side of the live view sends these through it.
pub trait ModuleMessage {
    fn heartbeat(heartbeat: LiveHeartbeat) -> Self;
    fn notice(notice: LiveNotice) -> Self;
}

/// What input the watched source takes, which the page sends it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LiveInputs {
    #[garde(skip)]
    pub pointer: PointerKind,
}

/// A point of the picture, in its coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LivePoint {
    #[garde(range(min = 0.0, max = 4096.0))]
    pub x: f64,
    #[garde(range(min = 0.0, max = 4096.0))]
    pub y: f64,
}

/// Whether the agent's input is being delivered to the watched source, and
/// where it last acted, which the view marks.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LiveActivity {
    #[garde(skip)]
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "LivePoint")]
    #[garde(dive)]
    pub point: Option<LivePoint>,
}
