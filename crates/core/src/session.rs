//! What a session shows beside its transcript (`runtime.md` § Input): its
//! phase, the messages waiting to run, and the human steers it accepted but
//! has not yet written.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{BlockId, ModelSelection, TurnId, UserContentBlock};

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
