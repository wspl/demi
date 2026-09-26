//! Uploads (`web-api.md` § Uploads and media): a file's bytes go to the
//! caller's blobs with one attachment record, and a send, steer or edit
//! frame names the upload by its id.

use demi_core::{BlobRef, MAX_SAFE_INTEGER, Timestamp};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::ids::AttachmentId;

/// The most bytes an upload holds.
pub const ATTACHMENT_MAX_BYTES: usize = 25 * 1024 * 1024;

/// `POST /attachments?name=`: the file's name, which decides whether the
/// answer carries a text file's opening, as a message with the file does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct UploadQuery {
    #[garde(length(chars, min = 1, max = 255))]
    pub name: String,
}

/// The answer of an upload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AttachmentAnswer {
    pub attachment: AttachmentDto,
}

/// An upload as the composer shows it: the media type the backend read from
/// the file's bytes when it recognizes them, else the one it was sent with,
/// and a text file's opening.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentDto {
    pub id: AttachmentId,
    pub media_type: String,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub size_bytes: u64,
    /// The bytes in the caller's blobs.
    pub sha256: BlobRef,
    pub created_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub snippet: Option<String>,
}
