//! What a session shows beside its transcript (`runtime.md` § Input): its
//! phase, the messages waiting to run, and the human steers it accepted but
//! has not yet written.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{BlockId, MAX_SAFE_INTEGER, ModelSelection, Nullable, TurnId, UserContentBlock};

/// What a session is doing, as clients see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    Idle,
    Running,
    Compacting,
}

serde_plain::derive_display_from_serialize!(SessionPhase);
serde_plain::derive_fromstr_from_deserialize!(SessionPhase);

/// How full the session's next request is, as compaction estimates it
/// (`compaction.md` § Context estimate), with the window its thresholds use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextUsage {
    /// The estimate of the next request, in tokens.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub tokens: u64,
    /// The window the thresholds use, in tokens; null for a model that
    /// reports none.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u64>")]
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub window: Option<u64>,
    /// The estimate from which a `compact` frame is taken; null when any is.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u64>")]
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub compact_from: Option<u64>,
}

impl ContextUsage {
    /// Whether the user may compact now (`compaction.md` § When compaction
    /// runs).
    pub fn admits_compaction(&self) -> bool {
        self.compact_from.is_none_or(|from| self.tokens >= from)
    }
}

/// A message waiting in the queue; the checkpoint keeps the queue. The page
/// derives the text it shows from the content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueuedMessage {
    /// The message's id, which becomes its turn's id.
    #[garde(skip)]
    pub id: TurnId,
    #[garde(dive)]
    pub content: Vec<UserContentBlock>,
}

/// A call the model is writing (`runtime.md` § Calls being written): the
/// provider opened it and has not handed it over whole. It is never saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct PendingCall {
    /// The call's tool-use ID, which its `tool_call` block will have.
    #[garde(length(min = 1))]
    pub tool_use_id: String,
    #[garde(length(min = 1))]
    pub tool_name: String,
    /// The call's `description` once the model has written that string
    /// whole; null until then.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "crate::Nullable<String>")]
    #[garde(skip)]
    pub description: Option<String>,
}

/// A human steer the session accepted but has not yet written into the
/// transcript. It is never saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct PendingSteer {
    /// The steer's id, which its `steer` block will have.
    #[garde(skip)]
    pub id: BlockId,
    /// The running turn that receives the steer.
    #[garde(skip)]
    pub turn_id: TurnId,
    /// The model selection when the steer was accepted.
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(dive)]
    pub content: Vec<UserContentBlock>,
}
