//! The live view's frames over a user stream: a four-byte big-endian length, a
//! kind, then the payload. The stream itself has no message boundaries; a
//! picture travels in chunks of at most 16 KiB.

use bytes::{BufMut, Bytes, BytesMut};
use serde::Serialize;

use demi_command_sdk::{Input, ResidentError};
use demi_live_view_protocol::{CONTROL_FRAME, FrameSplitter, SourceId, VIDEO_FRAME, VideoHeader};

use crate::stream::EncodedFrame;

pub use demi_live_view_protocol::Frame;

/// A stream the page ended inside a frame: a defect of the page, which ends
/// the view.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("the stream ended inside a live frame")]
    Unfinished,
}

/// Splits the page's bytes into frames as they arrive.
pub struct Reader {
    input: Input,
    frames: FrameSplitter,
}

impl Reader {
    pub fn new(input: Input) -> Self {
        Self {
            input,
            frames: FrameSplitter::default(),
        }
    }

    /// The next frame, or none once the page ended its side.
    pub async fn next(&mut self) -> Result<Option<Frame>, ResidentError> {
        loop {
            if let Some(frame) = self.frames.next().map_err(ResidentError::failed)? {
                return Ok(Some(frame));
            }
            match self.input.next().await? {
                Some(chunk) => self.frames.push(&chunk),
                None if self.frames.is_empty() => return Ok(None),
                None => return Err(ResidentError::failed(FrameError::Unfinished)),
            }
        }
    }
}

/// A frame of `kind` whose payload `write` appends, `payload_length` bytes.
pub fn framed(kind: u8, payload_length: usize, write: impl FnOnce(&mut BytesMut)) -> Bytes {
    let mut bytes = BytesMut::with_capacity(5 + payload_length);
    bytes.put_u32(u32::try_from(1 + payload_length).expect("live frames are bounded"));
    bytes.put_u8(kind);
    write(&mut bytes);
    bytes.freeze()
}

/// A control message to the page.
pub fn control(message: &impl Serialize) -> Bytes {
    let json = serde_json::to_vec(message).expect("live messages serialize");
    framed(CONTROL_FRAME, json.len(), |bytes| {
        bytes.extend_from_slice(&json)
    })
}

/// The chunks of a picture of `source` to the page, in the viewer's stream
/// generation; none for a frame too large to number its chunks, which no
/// encoder sends.
pub fn chunks(source: &SourceId, generation: u32, frame: &EncodedFrame) -> Option<Vec<Bytes>> {
    let pieces = frame.data.chunks(VideoHeader::CHUNK_DATA);
    let chunks = u16::try_from(pieces.len().max(1)).ok()?;
    let header = |chunk: u16| {
        let mut head = Vec::with_capacity(VideoHeader::BYTES);
        VideoHeader {
            source: source.clone(),
            generation,
            sequence: frame.sequence,
            chunk,
            chunks,
            key: frame.key,
            timestamp: frame.timestamp,
            width: frame.width,
            height: frame.height,
        }
        .write(&mut head);
        head
    };
    if frame.data.is_empty() {
        let head = header(0);
        return Some(vec![framed(VIDEO_FRAME, head.len(), |bytes| bytes.extend_from_slice(&head))]);
    }
    Some(
        pieces
            .zip(0..)
            .map(|(piece, chunk)| {
                let head = header(chunk);
                framed(VIDEO_FRAME, head.len() + piece.len(), |bytes| {
                    bytes.extend_from_slice(&head);
                    bytes.extend_from_slice(piece);
                })
            })
            .collect(),
    )
}
