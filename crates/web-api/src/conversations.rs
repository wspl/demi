//! Conversations as the browser creates, lists and follows them
//! (`web-api.md` § Conversation creation and Fork, § Sidebar mutations, read
//! state and page synchronization).

use demi_agent_protocol::{Failures, SubagentJob};
use demi_core::{Block, BlockId, MAX_SAFE_INTEGER, ModelSelection, Nullable, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::{double_option, unwrap_or_skip};

use crate::error::ErrorCode;
use crate::ids::{ConversationId, DeviceId, ProviderId, WorkspaceId};
use crate::query::StrictBool;
use crate::text::Trimmed;

/// The most characters (Unicode scalar values) a conversation's title has.
pub const TITLE_MAX: usize = 256;

/// The most items one batch changes.
pub const BATCH_MAX: usize = 100;

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
        #[garde(length(min = 1), pattern(r"^/"))]
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
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub read_revision: u64,
    pub target: ConversationTarget,
    /// Advances with every change of the conversation's execution context,
    /// such as a target switch.
    #[garde(range(max = MAX_SAFE_INTEGER))]
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
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
    /// Whether output is newer than the read revision.
    pub unread: bool,
    /// Whether the title has read every message the user sent, so asking
    /// for a new one could say nothing the last did not
    /// (`product.md` § Conversation titles).
    pub title_current: bool,
    /// Whether a title request of the conversation is in flight.
    pub title_generating: bool,
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
    #[garde(range(max = MAX_SAFE_INTEGER))]
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

/// `PATCH /conversations/:id`: the fields to change, each applied on its
/// own; an absent field stays as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationPatch {
    /// A rename: 1 to 256 characters after trimming.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Trimmed")]
    #[garde(length(chars, min = 1, max = TITLE_MAX))]
    pub title: Option<Trimmed>,
    /// Archive, or restore.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "bool")]
    #[garde(skip)]
    pub archived: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "bool")]
    #[garde(skip)]
    pub pinned: Option<bool>,
    /// The provider entry and model the conversation selects, or null for
    /// none.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
    #[schemars(with = "Option<ModelChoice>")]
    #[garde(dive)]
    pub model: Option<Option<ModelChoice>>,
    /// A switch of the conversation's execution target.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "ConversationTarget")]
    #[garde(dive)]
    pub target: Option<ConversationTarget>,
}

/// A provider entry of the user's scope and one of its models.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelChoice {
    #[garde(skip)]
    pub provider_id: ProviderId,
    #[garde(length(chars, min = 1))]
    pub model_id: String,
}

/// A field of a conversation patch, as its result names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PatchField {
    Title,
    Archived,
    Pinned,
    Model,
    Target,
}

/// How one field of a patch went: applied, or refused with the code and
/// the HTTP status it would answer alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum FieldResult {
    Applied {
        field: PatchField,
    },
    Failed {
        field: PatchField,
        code: ErrorCode,
        message: String,
        http_status: u16,
    },
}

/// The answer of a patch: the conversation as it is now, and each field's
/// result in the order they were applied, the archive first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConversationUpdate {
    pub conversation: ConversationSummary,
    pub results: Vec<FieldResult>,
}

/// `POST /conversations/batch`: up to 100 patches, each of one of the
/// caller's conversations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct ConversationBatch {
    #[garde(length(min = 1, max = BATCH_MAX), dive)]
    pub items: Vec<BatchItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct BatchItem {
    #[garde(skip)]
    pub id: ConversationId,
    #[garde(dive)]
    pub patch: ConversationPatch,
}

/// The answer of a batch: an outcome per item, in the order given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BatchAnswer {
    pub results: Vec<BatchResult>,
}

/// One item's outcome: its patch's answer, or why the item was refused,
/// such as a conversation the caller does not have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum BatchResult {
    Updated {
        id: ConversationId,
        conversation: ConversationSummary,
        results: Vec<FieldResult>,
    },
    Refused {
        id: ConversationId,
        code: ErrorCode,
        message: String,
    },
}

/// `POST /conversations/:id/title`: a generated title, asked of `model`, the
/// selection the composer shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct TitleRequest {
    #[garde(dive)]
    pub model: ModelSelection,
}

/// `POST /conversations/:id/fork`: the new conversation's id, chosen by the
/// browser, and the completed assistant text the history is kept through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForkRequest {
    #[garde(skip)]
    pub id: ConversationId,
    #[garde(skip)]
    pub block_id: BlockId,
}

/// The answer of a Fork: the new conversation and the complete model
/// selection it inherited, which the composer starts from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ForkAnswer {
    pub conversation: ConversationSummary,
    pub model: ModelSelection,
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
