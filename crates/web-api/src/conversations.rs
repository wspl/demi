//! Conversations as the browser creates, lists and follows them
//! (`web-api.md` § Conversation creation and Fork, § Sidebar mutations, read
//! state and page synchronization).

use demi_agent_protocol::{Failures, SubagentJob};
use demi_core::{Block, Nullable, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::ids::{ConversationId, DeviceId, ProviderId, WorkspaceId};
use crate::query::StrictBool;

/// Where a conversation's work runs (`sessions-and-targets.md` § Resolve a
/// target): the user's Cloud, a directory on a paired device, or a
/// workspace. A new conversation runs on the Cloud, in its session
/// directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ConversationTarget {
    Cloud {
        /// An absolute directory on the Cloud; the conversation's session
        /// directory without it.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(inner(length(min = 1), pattern(r"^/")))]
        path: Option<String>,
    },
    Device {
        #[garde(skip)]
        device_id: DeviceId,
        #[garde(length(min = 1))]
        path: String,
    },
    Workspace {
        #[garde(skip)]
        workspace_id: WorkspaceId,
    },
}

/// `POST /conversations`: the id the browser chose for a new conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct CreateConversation {
    #[garde(skip)]
    pub id: ConversationId,
}

/// A conversation as the browser lists it: its record, with the directory
/// its work runs in, its status and its output revision. What the backend
/// keeps for itself, such as the owner, stays out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub id: ConversationId,
    pub title: String,
    pub archived: bool,
    pub pinned: bool,
    /// The output revision the user last acknowledged.
    pub read_revision: u64,
    pub target: ConversationTarget,
    /// Advances with every change of the conversation's execution context,
    /// such as a target switch.
    pub context_version: u64,
    /// The provider entry the conversation last selected; null for none.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<ProviderId>")]
    pub provider_id: Option<ProviderId>,
    /// The model of that entry; null exactly when `providerId` is.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub model_id: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// The directory the conversation's work runs in, resolved by the backend
    /// for every kind of target, so the browser never derives it.
    pub cwd: String,
    pub status: ConversationStatus,
    /// The output revision: it advances with each saved change of output,
    /// never with the user's input alone.
    pub revision: u64,
    /// Whether output is newer than the read revision.
    pub unread: bool,
}

/// Where a conversation stands (`web-api.md` § Sidebar mutations, read state
/// and page synchronization): running or compacting while its live tree
/// works, interrupted when its last checkpoint was saved in a turn and no
/// session is live, otherwise what its latest terminal block says, or idle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConversationStatus {
    Running,
    Compacting,
    Interrupted,
    Error,
    Stopped,
    Completed,
    Idle,
}

/// `{ conversation }`: the answer of a create.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConversationAnswer {
    pub conversation: ConversationSummary,
}

/// `GET /conversations`: the caller's conversations in sidebar order,
/// pinned first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Conversations {
    pub conversations: Vec<ConversationSummary>,
}

/// `?archived=true|false`: the archived conversations, or the others.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct ConversationsQuery {
    #[serde(default)]
    pub archived: StrictBool,
}

/// `POST /conversations/:id/read`: the output revision the page showed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct ReadRequest {
    #[garde(range(max = demi_core::MAX_SAFE_INTEGER))]
    pub revision: u64,
}

/// `GET /conversations/:id/transcript`: the conversation's history as its
/// database holds it, the root's blocks and each subagent's, with the failure
/// facts of their error blocks (`backend.md` § Failure facts). Media travels
/// by blob reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub blocks: Vec<Block>,
    /// The facts of the root's error blocks, by block id; absent when none
    /// yields one.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Failures")]
    pub failures: Option<Failures>,
    /// Every subagent of the tree, in spawn order under each parent.
    pub subagents: Vec<SubagentHistory>,
}

/// One subagent's history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SubagentHistory {
    pub subagent: SubagentJob,
    pub blocks: Vec<Block>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Failures")]
    pub failures: Option<Failures>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_target_names_its_kind_and_refuses_what_its_kind_does_not_hold() {
        let cloud: ConversationTarget = demi_core::decode(r#"{"kind":"cloud"}"#).unwrap();
        assert_eq!(cloud, ConversationTarget::Cloud { path: None });
        assert_eq!(serde_json::to_value(&cloud).unwrap(), json!({ "kind": "cloud" }));
        let device = ConversationTarget::Device {
            device_id: DeviceId::try_from("laptop").unwrap(),
            path: "/work".into(),
        };
        assert_eq!(
            serde_json::to_value(&device).unwrap(),
            json!({ "kind": "device", "deviceId": "laptop", "path": "/work" })
        );
        for refused in [
            r#"{"kind":"cloud","path":"relative"}"#,
            r#"{"kind":"cloud","path":null}"#,
            r#"{"kind":"device","deviceId":"laptop","path":""}"#,
            r#"{"kind":"workspace","workspaceId":"w1","path":"/work"}"#,
            r#"{"kind":"elsewhere"}"#,
        ] {
            assert!(demi_core::decode::<ConversationTarget>(refused).is_err(), "{refused}");
        }
    }
}
