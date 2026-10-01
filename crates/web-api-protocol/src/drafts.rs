//! A conversation's draft (`web-api.md` § Conversation drafts): the message
//! its composer holds before it is sent, which the backend keeps so that
//! every page of the conversation shows it. A save names the revision its
//! text was built on and always takes effect; one built on an older revision
//! keeps the version it replaced, which a page restores or dismisses.

use demi_conversation_socket_protocol::ClientContent;
use demi_shared_types::{BlobRef, MAX_SAFE_INTEGER, Nullable};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::ids::AttachmentId;

/// The mark a draft's text holds where each file's capsule stands, the one
/// the composer's editor writes (`product.md` § Attachments).
pub const ATTACHMENT_MARK: char = '\u{FFFC}';

/// The most bytes a draft's text and files take as stored.
pub const DRAFT_BYTES_MAX: usize = 256 * 1024;

/// `PUT /conversations/:id/draft`: the message's Markdown and its files in
/// the order of their marks, each named as a frame names it, an `upload` or
/// a `remote_file`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DraftSave {
    /// The revision the page's text was built on.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub base: u64,
    #[garde(skip)]
    pub text: String,
    #[garde(dive)]
    pub files: Vec<ClientContent>,
}

/// What the page asks of the version a save replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReplacedAction {
    /// Exchange it with the draft, which then becomes the replaced version.
    Restore,
    /// Drop it.
    Dismiss,
}

/// `POST /conversations/:id/draft/replaced`: an action on the replaced
/// version the page shows, named by its revision.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate,
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplacedDraftAction {
    #[garde(skip)]
    pub action: ReplacedAction,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
}

/// The answer of every draft route: the draft as it now is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DraftAnswer {
    pub draft: ConversationDraft,
}

/// A conversation's draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDraft {
    /// How many times the draft changed: 0 before its first save, one more
    /// with every save, restore and dismissal.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
    /// The message's Markdown, with an attachment mark where each file's
    /// capsule stands.
    pub text: String,
    /// The files in the order of their marks.
    pub files: Vec<DraftFile>,
    /// The version a save replaced without having been built on it.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<ReplacedDraft>")]
    pub replaced: Option<ReplacedDraft>,
}

impl ConversationDraft {
    /// The draft of a conversation that never saved one.
    pub fn empty() -> Self {
        Self {
            revision: 0,
            text: String::new(),
            files: Vec::new(),
            replaced: None,
        }
    }
}

/// A version of the draft that a save replaced, with the revision it had as
/// the draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplacedDraft {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
    pub text: String,
    pub files: Vec<DraftFile>,
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
