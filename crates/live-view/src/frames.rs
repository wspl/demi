//! The live view's frames over a user stream (`live-view.md`
//! § Framing and versions): a four-byte big-endian length, a kind, then the
//! payload. The stream itself has no message boundaries.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use serde::Serialize;

use demi_command_sdk::{Input, ServiceError};
use demi_live_view_protocol::{
    CONTROL_FRAME, MAX_FRAME_BYTES, SourceId, VIDEO_FRAME, VideoHeader,
};

use crate::stream::EncodedFrame;

/// One frame the page sent: its kind and payload, which the source's
/// protocol decodes.
#[derive(Debug)]
pub struct Frame {
    pub kind: u8,
    pub payload: Bytes,
}

/// A frame the protocol refuses (`live-view.md` § Framing and versions): a
/// defect of the page, which ends the view.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("a live frame is empty or too large")]
    Length,
    #[error("the stream ended inside a live frame")]
    Unfinished,
}

/// Splits the page's bytes into frames as they arrive.
pub struct Reader {
    input: Input,
    pending: BytesMut,
}

impl Reader {
    pub fn new(input: Input) -> Self {
        Self {
            input,
            pending: BytesMut::new(),
        }
    }

    /// The next frame, or none once the page ended its side.
    pub async fn next(&mut self) -> Result<Option<Frame>, ServiceError> {
        loop {
            if self.pending.len() >= 4 {
                let length = u32::from_be_bytes(self.pending[..4].try_into().expect("four bytes"));
                let length = usize::try_from(length).unwrap_or(usize::MAX);
                if length == 0 || length > MAX_FRAME_BYTES {
                    return Err(ServiceError::failed(FrameError::Length));
                }
                if self.pending.len() >= 4 + length {
                    self.pending.advance(4);
                    let mut payload = self.pending.split_to(length).freeze();
                    let kind = payload.get_u8();
                    return Ok(Some(Frame { kind, payload }));
                }
            }
            match self.input.next().await? {
                Some(chunk) => self.pending.extend_from_slice(&chunk),
                None if self.pending.is_empty() => return Ok(None),
                None => return Err(ServiceError::failed(FrameError::Unfinished)),
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

/// A picture of `source` to the page, in the viewer's stream generation.
pub fn video(source: &SourceId, generation: u32, sequence: u32, frame: &EncodedFrame) -> Bytes {
    let header = VideoHeader {
        source: source.clone(),
        generation,
        sequence,
        key: frame.key,
        timestamp: frame.timestamp,
        width: frame.width,
        height: frame.height,
    };
    let mut head = Vec::with_capacity(VideoHeader::BYTES);
    header.write(&mut head);
    framed(VIDEO_FRAME, head.len() + frame.data.len(), |bytes| {
        bytes.extend_from_slice(&head);
        bytes.extend_from_slice(&frame.data);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_split_across_chunks_and_join_within_one() {
        let hello = br#"{"type":"hello","platform":"mac"}"#;
        let release = br#"{"type":"release"}"#;
        let mut bytes = BytesMut::new();
        for (kind, payload) in [(CONTROL_FRAME, &hello[..]), (CONTROL_FRAME, &release[..]), (3, &b"abc"[..])] {
            bytes.put_u32(1 + payload.len() as u32);
            bytes.put_u8(kind);
            bytes.put_slice(payload);
        }
        let bytes = bytes.freeze();
        // One byte at a time, then everything in one chunk.
        for chunks in [
            bytes
                .chunks(1)
                .map(Bytes::copy_from_slice)
                .collect::<Vec<_>>(),
            vec![bytes.clone()],
        ] {
            let input = Input::from_stream(futures_util::stream::iter(chunks.into_iter().map(Ok)));
            let mut reader = Reader::new(input);
            let mut frames = Vec::new();
            while let Some(frame) = reader.next().await.unwrap() {
                frames.push((frame.kind, frame.payload));
            }
            assert_eq!(
                frames,
                [
                    (CONTROL_FRAME, Bytes::from_static(hello)),
                    (CONTROL_FRAME, Bytes::from_static(release)),
                    (3, Bytes::from_static(b"abc")),
                ]
            );
        }
    }

    #[tokio::test]
    async fn a_frame_the_page_cannot_send_ends_the_stream() {
        for bytes in [vec![0, 0, 0, 0], vec![0, 0, 0, 5, CONTROL_FRAME]] {
            let input = Input::from_stream(futures_util::stream::iter([Ok(Bytes::from(bytes))]));
            assert!(Reader::new(input).next().await.is_err());
        }
        let mut too_large = vec![];
        too_large.extend_from_slice(&u32::try_from(MAX_FRAME_BYTES + 1).unwrap().to_be_bytes());
        let input = Input::from_stream(futures_util::stream::iter([Ok(Bytes::from(too_large))]));
        assert!(Reader::new(input).next().await.is_err());
    }
}
