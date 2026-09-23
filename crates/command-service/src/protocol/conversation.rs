//! Conversation lifecycle requests a trusted parent sends a resident service,
//! with invocation framing.

use serde::{Deserialize, Serialize};

/// Release a conversation's resources, or report which conversations hold
/// any. `Status` is a struct variant so that unknown fields are refused: serde
/// ignores them for a unit variant of an internally tagged enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(tag = "operation", rename_all = "lowercase", deny_unknown_fields)]
pub enum ConversationRequest {
    Release {
        #[garde(length(min = 1))]
        conversation: String,
    },
    Status {},
}

/// The conversations that hold resources in a service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct ConversationStatus {
    #[garde(inner(length(min = 1)))]
    pub conversations: Vec<String>,
}
