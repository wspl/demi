//! The browser's frames among the live view's (`live-view.md` § Framing and
//! versions): control messages in the browser's protocol, and the bytes of
//! files the viewer chose. The live view's reader splits the stream.

use bytes::Bytes;

use demi_command_package_browser_protocol::live::{FILE_FRAME, FileHeader, LiveViewerMessage};
use demi_command_sdk::{Input, ServiceError};
use demi_live_view::frames::{Frame, Reader as FrameReader};
use demi_live_view_protocol::CONTROL_FRAME;
use demi_shared_types::DecodeError;

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

/// A frame the browser's protocol refuses: a defect of the page, which ends
/// the view.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FrameError {
    #[error("invalid live message: {0}")]
    Message(DecodeError),
    #[error("invalid file frame: {0}")]
    File(DecodeError),
    #[error("unknown live frame kind {0}")]
    Kind(u8),
}

/// Reads the page's frames as the browser's messages.
pub(crate) struct Reader(FrameReader);

impl Reader {
    pub fn new(input: Input) -> Self {
        Self(FrameReader::new(input))
    }

    /// The next frame, or none once the page ended its side.
    pub async fn next(&mut self) -> Result<Option<Inbound>, ServiceError> {
        match self.0.next().await? {
            Some(frame) => decode(frame).map(Some),
            None => Ok(None),
        }
    }
}

fn decode(Frame { kind, payload }: Frame) -> Result<Inbound, ServiceError> {
    match kind {
        CONTROL_FRAME => LiveViewerMessage::decode(&payload)
            .map(Inbound::Control)
            .map_err(|error| ServiceError::failed(FrameError::Message(error))),
        FILE_FRAME => {
            let (header, data) = FileHeader::split(&payload)
                .map_err(|error| ServiceError::failed(FrameError::File(error)))?;
            Ok(Inbound::File {
                upload: header.upload,
                file: header.file,
                data: payload.slice_ref(data),
            })
        }
        kind => Err(ServiceError::failed(FrameError::Kind(kind))),
    }
}

#[cfg(test)]
mod tests {
    use bytes::{BufMut, BytesMut};

    use super::*;

    #[tokio::test]
    async fn the_browsers_frames_decode_as_its_messages_and_files() {
        let hello = br#"{"type":"hello","platform":"mac"}"#;
        let mut bytes = BytesMut::new();
        bytes.put_u32(1 + hello.len() as u32);
        bytes.put_u8(CONTROL_FRAME);
        bytes.put_slice(hello);
        bytes.put_u32(1 + 8 + 3);
        bytes.put_u8(FILE_FRAME);
        bytes.put_u32(7);
        bytes.put_u32(1);
        bytes.put_slice(b"abc");
        let input = Input::from_stream(futures_util::stream::iter([Ok(bytes.freeze())]));
        let mut reader = Reader::new(input);
        assert!(matches!(
            reader.next().await.unwrap(),
            Some(Inbound::Control(LiveViewerMessage::Hello(hello)))
                if hello.platform == demi_live_view_protocol::Platform::Mac
        ));
        assert!(matches!(
            reader.next().await.unwrap(),
            Some(Inbound::File { upload: 7, file: 1, data }) if data == Bytes::from_static(b"abc")
        ));
        assert!(reader.next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_frame_the_browser_cannot_take_ends_the_stream() {
        for bytes in [
            vec![0, 0, 0, 1, 9],
            vec![0, 0, 0, 3, CONTROL_FRAME, b'{', b'}'],
            vec![0, 0, 0, 2, FILE_FRAME, 0],
        ] {
            let input = Input::from_stream(futures_util::stream::iter([Ok(Bytes::from(bytes))]));
            assert!(Reader::new(input).next().await.is_err());
        }
    }
}
