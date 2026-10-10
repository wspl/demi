//! A command's media (`runtime.md` § Media the model views): the images,
//! videos and PDF documents its job viewed, as the backend holds them once
//! the command ended.

use bytes::Bytes;

/// One medium a command's job viewed, as the backend has it when the
/// command ended: its number in the command, its type and size, the facts
/// the runner announced with it, and its bytes or why the backend does not
/// have them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandMedium {
    pub number: u32,
    pub media_type: String,
    pub size: u64,
    /// A document's file name; none for one viewed from stdin.
    pub name: Option<String>,
    /// An image's or a video's size in pixels, from its header.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// A video's length, from its container's header.
    pub duration_ms: Option<u64>,
    pub bytes: Result<Bytes, String>,
}
