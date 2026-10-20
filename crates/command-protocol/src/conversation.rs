//! Conversation lifecycle requests a trusted parent sends a resident service,
//! with invocation framing.
//!
//! This is the one port through which state that outlives a shell job (the
//! conversation browser's tabs) attaches to the runtime, and deliberately the
//! only one.
//!
//! - The runtime provides the mechanism, the identity and the release, and no
//!   policy: it does not know what a package holds, why or for how long.
//! - The package keeps the state itself, keyed by the conversation name of the
//!   invocation's context, which a script cannot change, so it cannot select
//!   another conversation's state.
//! - The backend supplies the one policy, the conversation idle rule
//!   (`crates/backend-user-shard/src/lifecycle/mod.rs`).
//!
//! A package that needs more than this port shows that a generic capability is
//! missing here, not that the package deserves a path of its own through the
//! runner or backend.
//!
//! # Requests
//!
//! - `Status` answers at once from what the service holds, never waiting for a
//!   release or other work in progress.
//! - `Release` ends everything held for the conversation, awaits the package's
//!   cleanup and acknowledges. Releasing an unknown conversation is harmless.
//! - A service's shutdown releases every conversation it holds all at once, so
//!   the whole fits the shutdown deadline.
//! - A cleanup that fails retires the service as faulty and says so, never
//!   claiming a successful release.
//!
//! Only the runner sends these requests; the local command client cannot.

use serde::{Deserialize, Serialize};

use super::invocation::conversation_name;

/// Release a conversation's resources, or report which conversations hold
/// any. `Status` is a struct variant so that unknown fields are refused: serde
/// ignores them for a unit variant of an internally tagged enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(tag = "operation", rename_all = "lowercase", deny_unknown_fields)]
pub enum ConversationRequest {
    Release {
        #[garde(custom(conversation_name))]
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
