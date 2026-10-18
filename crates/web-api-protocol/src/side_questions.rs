//! A conversation's side questions (`comments-and-ask.md` § Side questions):
//! the list a page reads, asking one, a follow-up, a Retry and folding its
//! card. The answers themselves are the side question's transcript, which a
//! page reads by its pages with `?node=` and follows on the conversation's
//! socket, as a subagent's.

use demi_shared_types::{BlockId, MAX_SAFE_INTEGER, MediaSource, NodeId, Nullable, Timestamp, is_blank};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One side question: where it asks, what, and how its card shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
#[garde(allow_unvalidated)]
pub struct SideQuestion {
    /// The id the page chose, its node's: `?node=` reads its transcript.
    pub id: NodeId,
    /// The root's text block the asked words are in.
    pub block_id: BlockId,
    /// The asked words' character offsets in the block's text as the page
    /// renders it, as a comment's.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub start: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub end: u64,
    /// The asked words; a picture's alt text or name for a side question
    /// about a picture.
    pub quote: String,
    /// The picture a side question about a picture asks of, whose place
    /// among the block's pictures `start` and `end` are, as a comment on a
    /// picture's (`comments-and-ask.md` § Images); absent for words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<MediaSource>,
    /// The first question; a follow-up is a `user` block of its transcript.
    pub question: String,
    /// The small model's title; null until it wrote one, when the card says
    /// *Ask*.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub title: Option<String>,
    /// Whether its card is folded into the margin's mark.
    pub folded: bool,
    pub asked_at: Timestamp,
}

/// `GET /conversations/:id/side-questions`: every side question, in the
/// order they were asked, with the revision they were read at, which the
/// summary's `sideQuestionsRevision` is compared with (`web-api-sync.md`
/// § Revisions counted in memory).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct SideQuestions {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
    #[garde(dive)]
    pub side_questions: Vec<SideQuestion>,
}

/// `POST /conversations/:id/side-questions`: a new side question, under the
/// id the page chose, about the words `quote` of the completed assistant
/// text `blockId`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AskRequest {
    #[garde(length(chars, min = 1, max = 64))]
    pub id: String,
    #[garde(skip)]
    pub block_id: BlockId,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub start: u64,
    #[garde(range(max = MAX_SAFE_INTEGER), custom(after_start(&self.start)))]
    pub end: u64,
    #[garde(length(min = 1))]
    pub quote: String,
    /// The picture asked of, for a side question about a picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(dive)]
    pub picture: Option<MediaSource>,
    #[garde(custom(not_blank))]
    pub question: String,
}

/// `POST /conversations/:id/side-questions/:sideId/messages`: a follow-up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SideFollowUp {
    #[garde(custom(not_blank))]
    pub text: String,
}

/// `PATCH /conversations/:id/side-questions/:sideId`: folds its card into
/// the margin's mark, or opens it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SideQuestionPatch {
    #[garde(skip)]
    pub folded: bool,
}

/// The answer of a side question's creation and of its patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct SideQuestionAnswer {
    #[garde(dive)]
    pub side_question: SideQuestion,
}

fn after_start(start: &u64) -> impl FnOnce(&u64, &()) -> garde::Result + '_ {
    move |end, _| {
        if end > start {
            Ok(())
        } else {
            Err(garde::Error::new("ends after it starts"))
        }
    }
}

fn not_blank(text: &str, _: &()) -> garde::Result {
    if is_blank(text) {
        Err(garde::Error::new("holds no words"))
    } else {
        Ok(())
    }
}
