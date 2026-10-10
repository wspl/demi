//! Conversations as the web app creates, lists and follows them
//! (`web-api.md` § Conversation creation and Fork, § Sidebar mutations, read
//! state and page synchronization).

use demi_conversation_socket_protocol::{Failures, SubagentJob};
use demi_shared_types::{
    Block, BlockId, CommandId, InstructionEntry, MAX_SAFE_INTEGER, NodeId, Nullable, Timestamp,
};
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
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
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

/// `POST /conversations`: the id the web app chose for a new conversation,
/// and what it starts with, so creating it is one request: the fields
/// [`ConversationPatch`] takes except `archived` and `notifyAgent`. They
/// apply as part of the creation, and one that is refused refuses it.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate,
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateConversation {
    #[garde(skip)]
    pub id: ConversationId,
    /// Its title, as a rename gives it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "Trimmed")]
    #[garde(length(chars, min = 1, max = TITLE_MAX))]
    pub title: Option<Trimmed>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "bool")]
    #[garde(skip)]
    pub pinned: Option<bool>,
    /// Its model, as a switch names it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "ModelChoice")]
    #[garde(dive)]
    pub model: Option<ModelChoice>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(length(chars, min = 1))]
    pub thinking_effort: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    #[schemars(with = "Option<String>")]
    #[garde(length(chars, min = 1))]
    pub service_tier_id: Option<Option<String>>,
    /// Where its work runs; the Cloud when absent.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "ConversationTarget")]
    #[garde(dive)]
    pub target: Option<ConversationTarget>,
}

/// What `POST /conversations` answers: the conversation as the list shows
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CreatedConversation {
    pub conversation: ConversationSummary,
}

/// A conversation as the web app lists it: its record, with the directory
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
    /// The conversation's model settings, the value every page shows; null
    /// while the conversation has no model yet.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<ModelSettings>")]
    pub model: Option<ModelSettings>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// The directory the conversation's work runs in, resolved by the backend
    /// for every kind of target, so the web app never derives it.
    pub cwd: String,
    pub status: ConversationStatus,
    /// The output revision: it advances with each saved change of output,
    /// never with the user's input alone.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
    /// The root's latest ended turn; null before the first. A page notifies
    /// of a turn it has not seen end (`product.md` § Notifications).
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<LastTurn>")]
    pub last_turn: Option<LastTurn>,
    /// Whether output is newer than the read revision.
    pub unread: bool,
    /// Whether the title has read every message the user sent, so asking
    /// for a new one could say nothing the last did not
    /// (`product.md` § Conversation titles).
    pub title_current: bool,
    /// Whether a title request of the conversation is in flight.
    pub title_generating: bool,
    /// Whether the conversation's tree is open with commands of plugins the
    /// user has since turned on or off, so a reload would change them
    /// (`plugins.md` § A user's plugins).
    pub plugins_changed: bool,
    /// The revision of the conversation's draft, 0 before its first save
    /// (`web-api.md` § Conversation drafts): a page reads the draft only when
    /// this is higher than the revision it holds.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub draft_revision: u64,
    /// The revision of the conversation's work panel, 0 before its first
    /// change (`web-api.md` § Work panel state), read the same way.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub panel_revision: u64,
    /// The revision of the conversation's attached hosts, raised by an
    /// attach, a detach or a rename (`web-api.md` § Sidebar mutations and
    /// read state), read the same way.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub hosts_revision: u64,
    /// The revision of each plugin's conversation state, in registration
    /// order (`web-api.md` § Conversation state of plugins): a page reads a
    /// state only when its revision is newer than the one it holds, counted
    /// in memory (`web-api.md` § Revisions counted in memory).
    #[garde(dive)]
    pub plugin_revisions: Vec<PluginRevision>,
    /// How many permission requests of the conversation wait for the user,
    /// which the sidebar shows as the needs-you mark (`web-api.md`
    /// § Conversation permissions).
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub permission_requests: u64,
    /// Rises with each change of the conversation's permission requests in
    /// this run of the backend: a page reads them only when this is newer
    /// than the revision it holds (`web-api.md` § Revisions counted in
    /// memory).
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub permissions_revision: u64,
}

/// The latest ended turn of a conversation's root (`web-api.md` § Sidebar
/// mutations and read state).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LastTurn {
    /// The block that ended the turn, so a turn that a Resume continues
    /// ends anew.
    pub id: BlockId,
    pub outcome: TurnOutcome,
    /// The first [`ANSWER_START_CHARS`] characters (Unicode scalar values) of
    /// its last answer's Markdown; null when it gave none.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub answer_start: Option<String>,
}

/// How much of a turn's last answer [`LastTurn`] carries.
pub const ANSWER_START_CHARS: usize = 400;

/// How a turn ended: with an answer, with an error, or stopped by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Finished,
    Failed,
    Stopped,
}

/// The revision of one plugin's state for a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginRevision {
    #[garde(skip)]
    pub plugin: String,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
}

/// A conversation's model settings (`models.md` § A conversation's model
/// settings): the provider entry and model, the thinking effort and the
/// service tier. A new conversation starts with the user's last choice, which
/// is model settings too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelSettings {
    #[garde(skip)]
    pub provider_id: ProviderId,
    #[garde(length(chars, min = 1))]
    pub model_id: String,
    /// An effort the model lists, or `disabled` for thinking off; null only
    /// for a model that lists no efforts.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub thinking_effort: Option<String>,
    /// A tier the model lists, or null for the vendor's default.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub service_tier_id: Option<String>,
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
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate,
)]
#[serde(deny_unknown_fields)]
pub struct ReadRequest {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
}

/// `?node=&before=|after=|around=&edge=` of `GET /conversations/:id/transcript`
/// (`web-api.md` § Pages). Queries are the backend's alone and are not
/// emitted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptQuery {
    /// The subagent whose transcript the page is of; the root's without it.
    #[serde(default)]
    pub node: Option<NodeId>,
    #[serde(default)]
    pub before: Option<u32>,
    #[serde(default)]
    pub after: Option<u32>,
    #[serde(default)]
    pub around: Option<BlockId>,
    /// The block the page holds at `before` or `after`.
    #[serde(default)]
    pub edge: Option<BlockId>,
}

/// `?node=` of the block and command reads.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeQuery {
    #[serde(default)]
    pub node: Option<NodeId>,
}

/// One page of an agent's transcript (`web-api.md` § Pages): whole
/// requests, its blocks in their light form, with the failure facts of its
/// error blocks (`backend.md` § Failure facts).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptPage {
    /// The index of the page's first block.
    pub start: u32,
    /// How many blocks the transcript holds.
    pub length: u32,
    #[serde(with = "demi_shared_types::client_blocks")]
    #[schemars(with = "Vec<Block>")]
    pub blocks: Vec<Block>,
    /// The facts of the page's error blocks, by block id; absent when none
    /// yields one.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "Failures")]
    pub failures: Option<Failures>,
    /// The entries of the agent's newest instructions block, which the
    /// context card lists; empty before the first.
    pub instructions: Vec<InstructionEntry>,
    /// The summary size of each compaction marker's boundary, which its
    /// divider tells; the boundary may be on another page.
    pub summaries: Vec<CompactionSummary>,
}

/// A compaction marker of a page and its boundary's summary size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct CompactionSummary {
    #[garde(skip)]
    pub marker: BlockId,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub summary_tokens: u64,
}

/// `GET /conversations/:id/transcript/blocks/:blockId`: a block whole.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WholeBlock {
    #[serde(with = "demi_shared_types::client_block")]
    #[schemars(with = "Block")]
    pub block: Block,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "Failures")]
    pub failures: Option<Failures>,
}

/// `GET /conversations/:id/subagents`: every subagent the conversation has
/// had, in the order they started.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Subagents {
    pub subagents: Vec<SubagentJob>,
}

/// `GET /conversations/:id/commands/:commandId`: where the `shell` call that
/// started a command lies, which a report's title brings into view
/// (`web-api.md` § Subagents and commands).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandCall {
    pub command_id: CommandId,
    /// The subagent that ran it; null for the root.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<NodeId>")]
    pub subagent_id: Option<NodeId>,
    /// The call's block in that agent's transcript.
    pub block_id: BlockId,
}

/// `PATCH /conversations/:id`: the fields to change, each applied on its
/// own; an absent field stays as it is.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate,
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationPatch {
    /// A rename: 1 to 256 characters after trimming.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "Trimmed")]
    #[garde(length(chars, min = 1, max = TITLE_MAX))]
    pub title: Option<Trimmed>,
    /// Archive, or restore.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "bool")]
    #[garde(skip)]
    pub archived: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "bool")]
    #[garde(skip)]
    pub pinned: Option<bool>,
    /// A switch to this model, with the effort and the tier this patch
    /// names, and the model's first effort and the vendor's default tier for
    /// a part it leaves out.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "ModelChoice")]
    #[garde(dive)]
    pub model: Option<ModelChoice>,
    /// The conversation's thinking effort: one its model lists, or
    /// `disabled` for thinking off.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(length(chars, min = 1))]
    pub thinking_effort: Option<String>,
    /// The conversation's service tier: one its model lists, or null for the
    /// vendor's default.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    #[schemars(with = "Option<String>")]
    #[garde(length(chars, min = 1))]
    pub service_tier_id: Option<Option<String>>,
    /// A switch of the conversation's execution target.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "ConversationTarget")]
    #[garde(dive)]
    pub target: Option<ConversationTarget>,
    /// With `target`, tells the agent of the switch once it is made: the
    /// user's message wakes the root (`sessions-and-targets.md` § Switch the
    /// primary target).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[garde(custom(names_a_switch(&self.target)))]
    pub notify_agent: bool,
}

/// Only a switch can be told to the agent.
fn names_a_switch(
    target: &Option<ConversationTarget>,
) -> impl FnOnce(&bool, &()) -> garde::Result + '_ {
    move |notify, _| {
        if *notify && target.is_none() {
            return Err(garde::Error::new("notifyAgent tells the agent of a target switch, and the patch names none"));
        }
        Ok(())
    }
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
    ThinkingEffort,
    ServiceTierId,
    Target,
}

/// How one field of a patch went: applied, or refused with the code and
/// the HTTP status it would answer alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
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
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum BatchResult {
    Updated {
        id: ConversationId,
        conversation: Box<ConversationSummary>,
        results: Vec<FieldResult>,
    },
    Refused {
        id: ConversationId,
        code: ErrorCode,
        message: String,
    },
}

/// `POST /conversations/:id/fork`: the new conversation's id, chosen by the
/// web app, and the completed assistant text the history is kept through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForkRequest {
    #[garde(skip)]
    pub id: ConversationId,
    #[garde(skip)]
    pub block_id: BlockId,
}

/// The answer of a Fork: the new conversation, whose model settings are the
/// ones it inherited from its source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ForkAnswer {
    pub conversation: ConversationSummary,
}

#[cfg(test)]
mod tests {
    use super::*;

    // The entries a provider keeps on a block never leave the backend
    // (`claude-code.md` § The session a process resumes): a page whose
    // blocks hold them has the wire shape of one whose blocks hold none.
    #[test]
    fn a_page_reaches_the_page_without_the_entries_its_provider_kept() {
        let text = serde_json::json!({
            "type": "text", "id": "t1", "createdAt": "2026-09-21T14:13:20.000Z",
            "model": { "providerId": "stub", "model": { "id": "stub-model", "name": "Stub",
                "contextWindow": 1000, "outputLimit": null, "thinking": [], "acceptedExtensions": [] },
                "thinking": null, "serviceTierId": null },
            "text": "Hello",
        });
        let wire = serde_json::json!({
            "start": 0, "length": 1, "blocks": [text], "instructions": [], "summaries": [],
        });
        let mut page: TranscriptPage = serde_json::from_value(wire.clone()).unwrap();
        *page.blocks[0].entries_mut().unwrap() =
            vec![serde_json::json!({ "type": "assistant", "uuid": "kept" })];
        assert_eq!(serde_json::to_value(&page).unwrap(), wire);
    }

    #[test]
    fn a_target_refuses_what_its_kind_does_not_hold() {
        let cloud: ConversationTarget = demi_shared_types::decode(r#"{"kind":"cloud"}"#).unwrap();
        assert_eq!(cloud, ConversationTarget::Cloud { path: None });
        for refused in [
            r#"{"kind":"cloud","path":"relative"}"#,
            r#"{"kind":"cloud","path":null}"#,
            r#"{"kind":"device","deviceId":"laptop","path":""}"#,
            r#"{"kind":"workspace","workspaceId":"w1","path":"/work"}"#,
            r#"{"kind":"elsewhere"}"#,
        ] {
            assert!(
                demi_shared_types::decode::<ConversationTarget>(refused).is_err(),
                "{refused}"
            );
        }
    }
}
