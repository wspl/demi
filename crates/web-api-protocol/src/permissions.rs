//! A conversation's permission requests (`web-api.md` § Conversation
//! permissions, `permissions.md`): what a page reads to show the card above
//! the composer, and the decision it sends.

use demi_shared_types::{MAX_SAFE_INTEGER, Nullable, Timestamp};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

demi_shared_types::id!(
    /// A permission request's id, which the backend assigns when a command
    /// is refused.
    PermissionRequestId
);

/// A permission category as a page shows it: its id, and, while the user's
/// command set declares it, its action and description. A category no
/// longer declared carries its id alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
#[garde(allow_unvalidated)]
pub struct PermissionCategory {
    pub id: String,
    /// A lowercase verb phrase that completes "Allow this conversation to
    /// …", such as `manage skills`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// What a grant allows, including its reach beyond the conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The subagent that ran a refused command, by the number and the
/// description the model knows it by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
pub struct RequestingAgent {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub number: u64,
    #[garde(skip)]
    pub description: String,
}

/// One undecided request: the command an agent ran without the grant of its
/// category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    #[garde(skip)]
    pub id: PermissionRequestId,
    #[garde(dive)]
    pub category: PermissionCategory,
    /// The command line as the agent ran it, quoted for a POSIX shell.
    #[garde(skip)]
    pub command: String,
    /// The subagent that ran it; null for the root.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<RequestingAgent>")]
    #[garde(dive)]
    pub agent: Option<RequestingAgent>,
    #[garde(skip)]
    pub created_at: Timestamp,
}

/// `GET /conversations/:id/permissions`: the undecided requests, oldest
/// first, with the revision they were read at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
pub struct ConversationPermissions {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
    #[garde(dive)]
    pub requests: Vec<PermissionRequest>,
}

/// The user's answer to a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    /// Grants the category for the conversation, deciding each of its
    /// requests.
    Allow,
    /// Decides this request alone and remembers nothing.
    Deny,
}

/// `POST /conversations/:id/permissions/requests/:request`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct DecidePermission {
    #[garde(skip)]
    pub decision: PermissionDecision,
}
