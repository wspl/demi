//! The live protocol's frames over a user stream (`browser-live-view.md`
//! § Framing and versions): a four-byte big-endian length, a kind, then the
//! payload. The stream itself has no message boundaries.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use demi_command_service::{Input, ServiceError};

use super::super::protocol::{
    LIVE_CONTROL_FRAME, LIVE_FILE_FRAME, LIVE_FILE_HEADER_BYTES, LIVE_MAX_FRAME_BYTES,
    LIVE_VIDEO_FRAME, LIVE_VIDEO_HEADER_BYTES, LiveInbound, LiveOutbound,
};
use super::capture::Frame;

/// What the page sends.
#[derive(Debug)]
pub(crate) enum Inbound {
    Control(LiveInbound),
    /// Bytes of the `file`th file of an upload.
    File {
        upload: u32,
        file: u32,
        data: Bytes,
    },
}

/// Splits the page's bytes into frames as they arrive.
pub(crate) struct Reader {
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
    pub async fn next(&mut self) -> Result<Option<Inbound>, ServiceError> {
        loop {
            if self.pending.len() >= 4 {
                let length = u32::from_be_bytes(self.pending[..4].try_into().expect("four bytes"));
                let length = usize::try_from(length).unwrap_or(usize::MAX);
                if length == 0 || length > LIVE_MAX_FRAME_BYTES {
                    return Err(invalid("a live frame is empty or too large"));
                }
                if self.pending.len() >= 4 + length {
                    self.pending.advance(4);
                    let mut frame = self.pending.split_to(length).freeze();
                    return decode(&mut frame).map(Some);
                }
            }
            match self.input.next().await? {
                Some(chunk) => self.pending.extend_from_slice(&chunk),
                None if self.pending.is_empty() => return Ok(None),
                None => return Err(invalid("the stream ended inside a live frame")),
            }
        }
    }
}

fn decode(frame: &mut Bytes) -> Result<Inbound, ServiceError> {
    match frame.get_u8() {
        LIVE_CONTROL_FRAME => serde_json::from_slice(frame)
            .map(Inbound::Control)
            .map_err(|error| invalid(&format!("invalid live message: {error}"))),
        LIVE_FILE_FRAME if frame.len() >= LIVE_FILE_HEADER_BYTES => Ok(Inbound::File {
            upload: frame.get_u32(),
            file: frame.get_u32(),
            data: frame.clone(),
        }),
        _ => Err(invalid("unknown live frame")),
    }
}

fn invalid(message: &str) -> ServiceError {
    ServiceError::Handler(message.into())
}

fn framed(kind: u8, payload_length: usize, write: impl FnOnce(&mut BytesMut)) -> Bytes {
    let mut bytes = BytesMut::with_capacity(5 + payload_length);
    bytes.put_u32(u32::try_from(1 + payload_length).expect("live frames are bounded"));
    bytes.put_u8(kind);
    write(&mut bytes);
    bytes.freeze()
}

/// A control message to the page.
pub(crate) fn control(message: &LiveOutbound) -> Bytes {
    let json = serde_json::to_vec(message).expect("live messages serialize");
    framed(LIVE_CONTROL_FRAME, json.len(), |bytes| {
        bytes.extend_from_slice(&json)
    })
}

/// A video frame to the page, in the viewer's stream generation.
pub(crate) fn video(tab: &str, generation: u32, sequence: u32, frame: &Frame) -> Bytes {
    framed(
        LIVE_VIDEO_FRAME,
        LIVE_VIDEO_HEADER_BYTES + frame.data.len(),
        |bytes| {
            let mut id = [0_u8; 24];
            let length = tab.len().min(id.len());
            id[..length].copy_from_slice(&tab.as_bytes()[..length]);
            bytes.put_slice(&id);
            bytes.put_u32(generation);
            bytes.put_u32(sequence);
            bytes.put_u8(u8::from(frame.key));
            bytes.put_bytes(0, 3);
            bytes.put_f64(frame.timestamp);
            bytes.put_u16(frame.width);
            bytes.put_u16(frame.height);
            bytes.extend_from_slice(&frame.data);
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_split_across_chunks_and_join_within_one() {
        let hello = br#"{"type":"hello","platform":"mac"}"#;
        let release = br#"{"type":"release"}"#;
        let mut bytes = BytesMut::new();
        for (kind, payload) in [
            (LIVE_CONTROL_FRAME, &hello[..]),
            (LIVE_CONTROL_FRAME, &release[..]),
        ] {
            bytes.put_u32(1 + payload.len() as u32);
            bytes.put_u8(kind);
            bytes.put_slice(payload);
        }
        bytes.put_u32(1 + 8 + 3);
        bytes.put_u8(LIVE_FILE_FRAME);
        bytes.put_u32(7);
        bytes.put_u32(1);
        bytes.put_slice(b"abc");
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
            assert!(matches!(
                reader.next().await.unwrap(),
                Some(Inbound::Control(LiveInbound::Hello { platform })) if platform == "mac"
            ));
            assert!(matches!(
                reader.next().await.unwrap(),
                Some(Inbound::Control(LiveInbound::Release {}))
            ));
            assert!(matches!(
                reader.next().await.unwrap(),
                Some(Inbound::File { upload: 7, file: 1, data }) if data == Bytes::from_static(b"abc")
            ));
            assert!(reader.next().await.unwrap().is_none());
        }
    }

    #[tokio::test]
    async fn a_frame_the_page_cannot_send_ends_the_stream() {
        for bytes in [
            vec![0, 0, 0, 0],
            vec![0, 0, 0, 1, 9],
            vec![0, 0, 0, 3, LIVE_CONTROL_FRAME, b'{', b'}'],
            vec![0, 0, 0, 5, LIVE_CONTROL_FRAME],
        ] {
            let input = Input::from_stream(futures_util::stream::iter([Ok(Bytes::from(bytes))]));
            assert!(Reader::new(input).next().await.is_err());
        }
    }
}
