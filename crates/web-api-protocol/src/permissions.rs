//! A conversation's permission requests (`web-api.md` § Conversation
//! permissions, `permissions.md`): what a page reads to show the card above
//! the composer, and the decision it sends.

use demi_shared_types::{MAX_SAFE_INTEGER, Nullable, Timestamp};
use serde_with::rust::unwrap_or_skip;

use crate::credentials::{CredentialField, Keep, RequestedValues};
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

/// One undecided request of a conversation (`permissions.md` § Requests,
/// `credentials.md` § Requests and routes), by its `kind`: what its agent
/// asks of the user, the command it ran, and who ran it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum PermissionRequest {
    /// The grants of the categories a command lacked, one or more
    /// (`permissions.md` § Several categories).
    Permission {
        #[garde(skip)]
        id: PermissionRequestId,
        /// The categories the call lacked, ordered by id.
        #[garde(length(min = 1), dive)]
        categories: Vec<PermissionCategory>,
        /// The command line as the agent ran it, quoted for a POSIX shell.
        #[garde(skip)]
        command: String,
        /// The subagent that ran it; null for the root.
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<RequestingAgent>")]
        #[garde(dive)]
        agent: Option<RequestingAgent>,
        #[garde(skip)]
        created_at: Timestamp,
    },
    /// A credential to save: new, or new values of the saved one of that
    /// name, which has the same fields.
    Credential {
        #[garde(skip)]
        id: PermissionRequestId,
        #[garde(skip)]
        name: String,
        /// Why the agent asks; empty when it gave no reason.
        #[garde(skip)]
        reason: String,
        #[garde(dive)]
        fields: Vec<CredentialField>,
        /// How long the card offers to keep it at first.
        #[garde(skip)]
        keep: Keep,
        /// The command line as the agent ran it, quoted for a POSIX shell.
        #[garde(skip)]
        command: String,
        /// The subagent that ran it; null for the root.
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<RequestingAgent>")]
        #[garde(dive)]
        agent: Option<RequestingAgent>,
        #[garde(skip)]
        created_at: Timestamp,
    },
    /// The use of a saved credential in this conversation.
    Allowance {
        #[garde(skip)]
        id: PermissionRequestId,
        #[garde(skip)]
        name: String,
        /// The user removed the credential since it was asked for.
        #[garde(skip)]
        removed: bool,
        /// The command line as the agent ran it, quoted for a POSIX shell.
        #[garde(skip)]
        command: String,
        /// The subagent that ran it; null for the root.
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<RequestingAgent>")]
        #[garde(dive)]
        agent: Option<RequestingAgent>,
        #[garde(skip)]
        created_at: Timestamp,
    },
}

impl PermissionRequest {
    pub fn id(&self) -> &PermissionRequestId {
        match self {
            Self::Permission { id, .. } | Self::Credential { id, .. } | Self::Allowance { id, .. } => id,
        }
    }

    /// The command line as the agent ran it.
    pub fn command(&self) -> &str {
        match self {
            Self::Permission { command, .. }
            | Self::Credential { command, .. }
            | Self::Allowance { command, .. } => command,
        }
    }

    /// The subagent that ran it; none for the root.
    pub fn agent(&self) -> Option<&RequestingAgent> {
        match self {
            Self::Permission { agent, .. }
            | Self::Credential { agent, .. }
            | Self::Allowance { agent, .. } => agent.as_ref(),
        }
    }
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
    /// Grants the request's categories for the conversation, deciding each
    /// request whose categories are then all granted.
    Allow,
    /// Decides this request alone and remembers nothing.
    Deny,
}

/// `POST /conversations/:id/permissions/requests/:request`: the decision,
/// and an allow of a credential request carries the credential's values
/// and how long to keep it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct DecidePermission {
    #[garde(skip)]
    pub decision: PermissionDecision,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "RequestedValues")]
    #[garde(dive)]
    pub credential: Option<RequestedValues>,
}
