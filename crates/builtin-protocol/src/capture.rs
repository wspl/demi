//! The capture extension's messages (`live-view.md` § Capture): the module's
//! commands and the extension's events travel as JSON text over the
//! extension's WebSocket, and each encoded frame as a binary message that
//! starts with [`FrameHeader`].

use serde::{Deserialize, Serialize};

use crate::DecodeError;

/// What the module tells the extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum CaptureCommand {
    /// Capture the tab of CDP target `target` at `width` × `height` pixels.
    Start {
        capture: u32,
        target: String,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
    },
    /// The consumer has the frames up to `sequence`; up to `window` more may
    /// be in flight.
    Ack {
        capture: u32,
        sequence: u32,
        window: u32,
    },
    /// Encode the next frame as a key frame.
    Keyframe { capture: u32 },
    Encoding {
        capture: u32,
        bitrate: u32,
        fps: u32,
    },
    Stop { capture: u32 },
}

/// What the extension tells the module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub enum CaptureEvent {
    /// The connection is open and takes commands.
    Ready {},
    /// The capture's first picture is on its way.
    Started { capture: u32 },
    /// No picture arrived: the page stopped painting before capture began.
    Stalled { capture: u32 },
    /// The capture ended as the module asked.
    Stopped { capture: u32 },
    /// The capture failed and the extension released it.
    Error { capture: u32, message: String },
}

impl CaptureEvent {
    /// Decodes a text message from the extension.
    pub fn decode(text: &str) -> Result<Self, DecodeError> {
        crate::decode_slice(text.as_bytes())
    }
}

/// An encoded frame's header, big-endian: the capture (u32), the sequence
/// number (u32), flags (u8, 1 = key frame), three reserved bytes, the
/// timestamp in microseconds (f64), the picture's width and height in pixels
/// (u16 each), and eight reserved bytes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameHeader {
    pub capture: u32,
    pub sequence: u32,
    pub key: bool,
    pub timestamp: f64,
    pub width: u16,
    pub height: u16,
}

impl FrameHeader {
    pub const BYTES: usize = 32;

    /// Splits a binary message into its header and the H.264 data after it.
    pub fn split(message: &[u8]) -> Result<(Self, &[u8]), DecodeError> {
        let Some((header, data)) = message.split_first_chunk::<{ Self::BYTES }>() else {
            return Err(DecodeError::Invalid(format!(
                "a capture frame of {} bytes is shorter than its header",
                message.len()
            )));
        };
        let field = |start: usize, end: usize| &header[start..end];
        let header = Self {
            capture: u32::from_be_bytes(field(0, 4).try_into().expect("four bytes")),
            sequence: u32::from_be_bytes(field(4, 8).try_into().expect("four bytes")),
            key: header[8] & 1 == 1,
            timestamp: f64::from_be_bytes(field(12, 20).try_into().expect("eight bytes")),
            width: u16::from_be_bytes(field(20, 22).try_into().expect("two bytes")),
            height: u16::from_be_bytes(field(22, 24).try_into().expect("two bytes")),
        };
        Ok((header, data))
    }
}
