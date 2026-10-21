//! A conversation's messages saved for later: messages the user keeps to send later,
//! which nothing sends but the user. Each holds its text and files as a draft holds them,
//! and no comments: they point at the replies of the conversation it was written in, so
//! saving the composer's message leaves them waiting in the draft.

use demi_conversation_socket_protocol::ClientContent;
use demi_shared_types::{MAX_SAFE_INTEGER, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::drafts::{ConversationDraft, DraftFile, Revisions};
use crate::ids::SavedMessageId;

/// The most messages a conversation keeps saved for later.
pub const SAVED_MAX: usize = 200;

/// `POST /conversations/:id/saved/from-draft`: the composer's message, as a
/// draft's `PUT` names it, with the revision its text was built on and the id
/// the page gives the new message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedFromDraft {
    /// The new message's id, which the page chooses.
    #[garde(skip)]
    pub id: SavedMessageId,
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
    /// The new message's id, which the page chooses.
    #[garde(skip)]
    pub id: SavedMessageId,
    #[garde(skip)]
    pub text: String,
    #[garde(dive)]
    pub files: Vec<ClientContent>,
}

/// `PUT /conversations/:id/saved/:messageId`, as an edit's Save sends it:
/// the message's new Markdown and its files in the order of their marks, as
/// a draft's `PUT` names them. The message keeps its place in the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedWrite {
    #[garde(skip)]
    pub text: String,
    #[garde(dive)]
    pub files: Vec<ClientContent>,
}

/// A message saved for later, whole, as `GET .../saved/:messageId` answers it.
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

/// How many characters of a saved message's text its listing carries.
pub const SAVED_OPENING_CHARS: usize = 300;

/// The messages a conversation keeps saved for later as the composer lists
/// them, newest first, with the list's revision: each carries its text's
/// opening, never the whole of it, since a message may hold up to a draft's
/// size (`docs/decisions/0029-a-conversation-opens-whole.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SavedListing {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
    pub messages: Vec<ListedSavedMessage>,
}

/// A message saved for later as the composer lists it. Its whole text is
/// `GET /conversations/:id/saved/:messageId`, read when the message is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListedSavedMessage {
    pub id: String,
    /// The text's first [`SAVED_OPENING_CHARS`] characters, the whole text
    /// when it is no longer.
    pub opening: String,
    /// How many characters the whole text holds.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub length: u64,
    /// The files in the order of their marks.
    pub files: Vec<DraftFile>,
    pub saved_at: Timestamp,
}

/// `POST /conversations/:id/saved`'s and `PUT .../saved/:messageId`'s
/// answer: the message as saved, with the revisions after it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SavedAnswer {
    pub message: SavedMessage,
    pub revisions: Revisions,
}

/// `POST /conversations/:id/saved/from-draft`'s answer: the message as
/// saved and the draft it emptied, with the revisions after them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FromDraftAnswer {
    pub message: SavedMessage,
    pub draft: ConversationDraft,
    pub revisions: Revisions,
}

/// `DELETE /conversations/:id/saved/:messageId`'s answer, whether the
/// message was there or gone already: the revisions after it, and the
/// message as it was, whole, which the page's Undo saves again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SavedRemoved {
    pub revisions: Revisions,
    /// The message the request removed; absent when it was gone already.
    pub message: Option<SavedMessage>,
}
