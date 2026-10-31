//! A reply the user is likely to send after an answer: words the small model
//! wrote in the user's voice, which the page offers under the answer and a
//! tap sends (`crates/agent-server/src/suggested_replies.rs`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One suggested reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SuggestedReply {
    /// The words a tap sends, one line.
    #[garde(length(chars, min = 1))]
    pub text: String,
    #[garde(skip)]
    pub kind: SuggestedReplyKind,
}

/// What a suggested reply does with the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SuggestedReplyKind {
    /// Answers what the answer asked or offered: an approval, a choice among
    /// its options, a reply to its question.
    Answer,
    /// Raises something new the user may want next.
    Topic,
}
