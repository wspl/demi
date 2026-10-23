//! The browser's frames among the live view's: control messages in the
//! browser's protocol, acknowledgements of the video chunks, and the bytes of
//! files the viewer chose. The live view's reader splits the stream.

use bytes::Bytes;

use demi_command_package_browser_protocol::live::{FILE_FRAME, FileHeader, LiveViewerMessage};
use demi_command_sdk::{Input, ResidentError};
use demi_live_view::frames::{Frame, Reader as FrameReader};
use demi_live_view_protocol::{ACK_FRAME, CONTROL_FRAME, ChunkAck};
use demi_shared_types::DecodeError;

/// What the page sends.
#[derive(Debug)]
pub(crate) enum Inbound {
    Control(LiveViewerMessage),
    Acks(Vec<ChunkAck>),
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
    #[error("invalid acknowledgements: {0}")]
    Acks(DecodeError),
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
    pub async fn next(&mut self) -> Result<Option<Inbound>, ResidentError> {
        match self.0.next().await? {
            Some(frame) => decode(frame).map(Some),
            None => Ok(None),
        }
    }
}

fn decode(Frame { kind, payload }: Frame) -> Result<Inbound, ResidentError> {
    match kind {
        CONTROL_FRAME => LiveViewerMessage::decode(&payload)
            .map(Inbound::Control)
            .map_err(|error| ResidentError::failed(FrameError::Message(error))),
        ACK_FRAME => ChunkAck::split(&payload)
            .map(Inbound::Acks)
            .map_err(|error| ResidentError::failed(FrameError::Acks(error))),
        FILE_FRAME => {
            let (header, data) = FileHeader::split(&payload)
                .map_err(|error| ResidentError::failed(FrameError::File(error)))?;
            Ok(Inbound::File {
                upload: header.upload,
                file: header.file,
                data: payload.slice_ref(data),
            })
        }
        kind => Err(ResidentError::failed(FrameError::Kind(kind))),
    }
}
