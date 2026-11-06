//! What the product reads from a device's files for the web app: directory
//! listings and file text. Text is what edit tracking and line counts read as
//! text, UTF-8 without a NUL byte, up to the size an edit snapshot keeps, so a
//! file the runner counted lines for is one the web app shows.

use bytes::BytesMut;
use demi_command_protocol::EDIT_FILE_BYTES;
pub use demi_command_protocol::{TextRefusal, text_of};
use demi_runner_client::RemoteDevice;
use demi_device_interface::{ByteRange, ByteStream, FileKind, DeviceError, LinkTarget};
use demi_runner_protocol::wire::{self, ReadAt};
use futures_util::StreamExt as _;
use demi_web_api_protocol::files::DirectoryEntry;

/// Why a file's text could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TextError {
    #[error(transparent)]
    Device(#[from] DeviceError),
    #[error(transparent)]
    Refused(#[from] TextRefusal),
}

/// The range a text read asks for: one byte more than the limit, so a file
/// whose bytes go past it is refused once that byte arrives.
pub const TEXT_RANGE: ByteRange = ByteRange {
    offset: 0,
    length: Some(EDIT_FILE_BYTES as u64 + 1),
};

/// The bytes of a [`TEXT_RANGE`] read as text, under the limits above.
pub async fn text_of_stream(mut stream: ByteStream) -> Result<String, TextError> {
    let mut bytes = BytesMut::new();
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk?);
        if bytes.len() > EDIT_FILE_BYTES {
            return Err(TextRefusal::TooLarge.into());
        }
    }
    Ok(text_of(Vec::from(bytes))?)
}

/// A directory's entries with their metadata, which the listing carries,
/// so the whole answer is one request to the device (`crates/runner-device/src/lib.rs`), and the device's number before it listed.
pub async fn browse_directory(
    device: &RemoteDevice,
    path: &str,
) -> Result<(Vec<DirectoryEntry>, ReadAt), DeviceError> {
    let (entries, read_at) = device.list_directory(path).await?;
    let entries = entries
        .into_iter()
        .map(|entry| DirectoryEntry {
            name: entry.name,
            is_directory: entry.kind == FileKind::Directory,
            size: entry.size,
            modified_at: entry.modified,
            link: entry.link.map(|link| wire::SymbolicLink {
                target: link.target,
                leads_to: match link.leads_to {
                    LinkTarget::File => wire::LinkTarget::File,
                    LinkTarget::Directory => wire::LinkTarget::Directory,
                    LinkTarget::Other => wire::LinkTarget::Other,
                    LinkTarget::Missing => wire::LinkTarget::Missing,
                    LinkTarget::Unreadable { reason } => wire::LinkTarget::Unreadable { reason },
                },
            }),
        })
        .collect();
    Ok((entries, read_at))
}
