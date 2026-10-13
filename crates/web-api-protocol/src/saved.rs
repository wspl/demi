//! A conversation's messages saved for later (`web-api.md` § Messages saved
//! for later): messages the user keeps to send later, which nothing sends
//! but the user. Each holds its text and files as a draft holds them.

use demi_conversation_socket_protocol::ClientContent;
use demi_shared_types::{MAX_SAFE_INTEGER, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::drafts::{ConversationDraft, DraftFile};

/// The most messages a conversation keeps saved for later.
pub const SAVED_MAX: usize = 200;

/// `POST /conversations/:id/saved/from-draft`: the composer's message, as a
/// draft's `PUT` names it, with the revision its text was built on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedFromDraft {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub base: u64,
    #[garde(skip)]
    pub text: String,
    #[garde(dive)]
    pub files: Vec<ClientContent>,
}

/// `POST /conversations/:id/saved`, as Undo after a removal sends it: the
/// message's Markdown and its files in the order of their marks, as a
/// draft's `PUT` names them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedSave {
    #[garde(skip)]
    pub text: String,
    #[garde(dive)]
    pub files: Vec<ClientContent>,
}

/// A message saved for later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SavedMessage {
    pub id: String,
    /// The message's Markdown, with an attachment mark where each file's
    /// capsule stands.
    pub text: String,
    /// The files in the order of their marks.
    pub files: Vec<DraftFile>,
    pub saved_at: Timestamp,
}

/// `GET /conversations/:id/saved`: the list, newest first, with its
/// revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SavedList {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
    pub messages: Vec<SavedMessage>,
}

/// `POST /conversations/:id/saved`'s answer: the message as saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SavedAnswer {
    pub message: SavedMessage,
}

/// `POST /conversations/:id/saved/from-draft`'s answer: the message as
/// saved and the draft it emptied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FromDraftAnswer {
    pub message: SavedMessage,
    pub draft: ConversationDraft,
}
