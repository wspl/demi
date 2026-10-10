//! A command's media (`runtime.md` § Media the model views): the images,
//! videos and PDF documents its job viewed, as the backend holds them once
//! the command ended.

use bytes::Bytes;

/// One medium a command's job viewed, as the backend has it when the
/// command ended: its number in the command, its type and size, and its
/// bytes or why the backend does not have them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandMedium {
    pub number: u32,
    pub media_type: String,
    pub size: u64,
    pub bytes: Result<Bytes, String>,
}
