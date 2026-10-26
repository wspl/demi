//! A conversation's draft: the message its composer holds before it is sent, which the
//! backend keeps so that every page of the conversation shows it. The draft is the last one
//! saved, whichever page saved it.

use demi_conversation_lane_protocol::ClientContent;
use demi_shared_types::{BlobRef, MAX_SAFE_INTEGER, MessageComment};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::ids::AttachmentId;

/// The mark a draft's text holds where each file's capsule stands, the one
/// the composer's editor writes.
pub const ATTACHMENT_MARK: char = '\u{FFFC}';

/// `PUT /conversations/:id/draft`: the message's Markdown and its files in
/// the order of their marks, each named as a frame names it, an `upload` or
/// a `remote_file`, and the comments waiting to be sent with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DraftSave {
    #[garde(skip)]
    pub text: String,
    #[garde(dive)]
    pub files: Vec<ClientContent>,
    #[garde(dive)]
    pub comments: Vec<MessageComment>,
}

/// `GET /conversations/:id/draft`'s answer: the draft as it now is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DraftAnswer {
    pub draft: ConversationDraft,
}

/// The revisions of a conversation's draft and of its messages saved for
/// later after a write, which every answer of a write of either carries:
/// the page holds the two as one piece of state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Revisions {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub draft: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub saved: u64,
}

/// The answer of a write that changes the draft, a save or an edit of a
/// saved message: the draft as the write left it, with the revisions after
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DraftWritten {
    pub draft: ConversationDraft,
    pub revisions: Revisions,
}

/// A conversation's draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDraft {
    /// How many times the draft changed: 0 before its first save, one more
    /// with every save.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
    /// The message's Markdown, with an attachment mark where each file's
    /// capsule stands.
    pub text: String,
    /// The files in the order of their marks.
    pub files: Vec<DraftFile>,
    /// The comments waiting to be sent, in the order they were saved.
    pub comments: Vec<MessageComment>,
}

impl ConversationDraft {
    /// The draft of a conversation that never saved one.
    pub fn empty() -> Self {
        Self {
            revision: 0,
            text: String::new(),
            files: Vec::new(),
            comments: Vec::new(),
        }
    }
}

/// A file of a draft as every page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DraftFile {
    /// An upload, with what its record holds: the media type the backend
    /// read, where its bytes are in the caller's blobs, and a text file's
    /// opening.
    Upload {
        r#ref: AttachmentId,
        file_name: String,
        media_type: String,
        sha256: BlobRef,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "String")]
        snippet: Option<String>,
    },
    /// A file on a paired device, which the model reads when it runs.
    RemoteFile { device_id: String, path: String },
}
