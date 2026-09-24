//! The live protocol's frames over a user stream (`live-view.md`
//! § Framing and versions): a four-byte big-endian length, a kind, then the
//! payload. The stream itself has no message boundaries.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use demi_builtin_protocol::{
    DecodeError,
    browser::TabId,
    live::{
        CONTROL_FRAME, FILE_FRAME, FileHeader, LiveModuleMessage, LiveViewerMessage,
        MAX_FRAME_BYTES, VIDEO_FRAME, VideoHeader,
    },
};
use demi_command_service::{Input, ServiceError};

use super::capture::Frame;

/// What the page sends.
#[derive(Debug)]
pub(crate) enum Inbound {
    Control(LiveViewerMessage),
    /// Bytes of the `file`th file of an upload.
    File {
        upload: u32,
        file: u32,
        data: Bytes,
    },
}

/// A frame the protocol refuses (`live-view.md` § Framing and versions): a
/// defect of the page, which ends the view.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FrameError {
    #[error("a live frame is empty or too large")]
    Length,
    #[error("the stream ended inside a live frame")]
    Unfinished,
    #[error("invalid live message: {0}")]
    Message(DecodeError),
    #[error("invalid file frame: {0}")]
    File(DecodeError),
    #[error("unknown live frame kind {0}")]
    Kind(u8),
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
                if length == 0 || length > MAX_FRAME_BYTES {
                    return Err(ServiceError::failed(FrameError::Length));
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
                None => return Err(ServiceError::failed(FrameError::Unfinished)),
            }
        }
    }
}

fn decode(frame: &mut Bytes) -> Result<Inbound, ServiceError> {
    match frame.get_u8() {
        CONTROL_FRAME => LiveViewerMessage::decode(frame)
            .map(Inbound::Control)
            .map_err(|error| ServiceError::failed(FrameError::Message(error))),
        FILE_FRAME => {
            let (header, data) = FileHeader::split(frame)
                .map_err(|error| ServiceError::failed(FrameError::File(error)))?;
            Ok(Inbound::File {
                upload: header.upload,
                file: header.file,
                data: frame.slice_ref(data),
            })
        }
        kind => Err(ServiceError::failed(FrameError::Kind(kind))),
    }
}

fn framed(kind: u8, payload_length: usize, write: impl FnOnce(&mut BytesMut)) -> Bytes {
    let mut bytes = BytesMut::with_capacity(5 + payload_length);
    bytes.put_u32(u32::try_from(1 + payload_length).expect("live frames are bounded"));
    bytes.put_u8(kind);
    write(&mut bytes);
    bytes.freeze()
}

/// A control message to the page.
pub(crate) fn control(message: &LiveModuleMessage) -> Bytes {
    let json = serde_json::to_vec(message).expect("live messages serialize");
    framed(CONTROL_FRAME, json.len(), |bytes| bytes.extend_from_slice(&json))
}

/// A video frame to the page, in the viewer's stream generation.
pub(crate) fn video(tab: &TabId, generation: u32, sequence: u32, frame: &Frame) -> Bytes {
    let header = VideoHeader {
        tab: tab.clone(),
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
        for (kind, payload) in [
            (CONTROL_FRAME, &hello[..]),
            (CONTROL_FRAME, &release[..]),
        ] {
            bytes.put_u32(1 + payload.len() as u32);
            bytes.put_u8(kind);
            bytes.put_slice(payload);
        }
        bytes.put_u32(1 + 8 + 3);
        bytes.put_u8(FILE_FRAME);
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
                Some(Inbound::Control(LiveViewerMessage::Hello { platform }))
                    if platform == demi_builtin_protocol::live::Platform::Mac
            ));
            assert!(matches!(
                reader.next().await.unwrap(),
                Some(Inbound::Control(LiveViewerMessage::Release {}))
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
            vec![0, 0, 0, 3, CONTROL_FRAME, b'{', b'}'],
            vec![0, 0, 0, 5, CONTROL_FRAME],
        ] {
            let input = Input::from_stream(futures_util::stream::iter([Ok(Bytes::from(bytes))]));
            assert!(Reader::new(input).next().await.is_err());
        }
    }
}
