//! A conversation's side questions: asking one, a follow-up, a Retry and
//! folding its card. Each is an agent of the conversation, whose entry the
//! conversation's agents list (`crates/conversation-socket-protocol/src/agents.rs`)
//! and whose answers are its transcript, which a page reads and follows as any
//! agent's.

use demi_conversation_socket_protocol::SideQuestion;
use demi_shared_types::{BlockId, MAX_SAFE_INTEGER, MediaSource, NodeId, TurnId, is_blank};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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

/// `POST /conversations/:id/side-questions/:sideId/messages`: a follow-up,
/// under the id the page chose, which names the wait it begins (the agent
/// entry's `waitAction`); the same request again sends nothing more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SideFollowUp {
    #[garde(skip)]
    pub id: TurnId,
    #[garde(custom(not_blank))]
    pub text: String,
}

/// `POST /conversations/:id/side-questions/:sideId/retry`: runs its failed
/// turn again, as the action `id` the page chose, which names the wait it
/// begins (the agent entry's `waitAction`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SideRetry {
    #[garde(skip)]
    pub id: TurnId,
}

/// `PATCH /conversations/:id/side-questions/:sideId`: folds its card into
/// the margin's mark, or opens it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SideQuestionPatch {
    #[garde(skip)]
    pub folded: bool,
}

/// The answer of a side question's creation and of its patch: the side
/// question as its agent entry shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct SideQuestionAnswer {
    #[garde(skip)]
    pub id: NodeId,
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
