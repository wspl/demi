//! A command's media (`runtime.md` § Media a command returns): the images
//! and videos its declared commands returned to the job, as the backend
//! holds them once the command ended, and as the conversation stores them
//! with the command's output.

use bytes::Bytes;
use demi_shared_types::BlobRef;
use serde::{Deserialize, Serialize};

/// One medium a command returned, as the backend has it when the command
/// ended: its number in the command, its type and size, and its bytes or
/// why the backend does not have them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandMedium {
    pub number: u32,
    pub media_type: String,
    pub size: u64,
    pub bytes: Result<Bytes, String>,
}

/// One medium as the conversation stores it with its command's output
/// (`storage.md` § Command outputs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredMedium {
    #[garde(range(min = 1))]
    pub number: u32,
    #[garde(length(min = 1))]
    pub media_type: String,
    #[garde(skip)]
    pub size: u64,
    #[garde(dive)]
    pub kept: MediumKept,
}

/// Whether the backend holds a medium: its blob, or why not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MediumKept {
    /// A blob reference is checked as it is read.
    Stored {
        #[garde(skip)]
        blob: BlobRef,
    },
    Missing {
        #[garde(length(min = 1))]
        reason: String,
    },
}
