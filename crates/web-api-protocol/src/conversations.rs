//! Conversations as the web app creates, lists and follows them.

use demi_conversation_socket_protocol::{
    AgentCost, AgentEntry, BlockVersions, ConversationAttachment, Failures, TurnSpeed, Version,
};
use demi_shared_types::{
    Block, BlockId, CommandId, InstructionEntry, MAX_SAFE_INTEGER, NodeId, Nullable, Timestamp,
    WorkflowChoice,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::{double_option, unwrap_or_skip};

use crate::error::ErrorCode;
use crate::ids::{ConversationId, DeviceId, ProviderId, WorkspaceId};
use crate::query::StrictBool;
use crate::drafts::ConversationDraft;
use crate::hosts::ConversationHosts;
use crate::panel::PanelRead;
use crate::permissions::ConversationPermissions;
use crate::saved::SavedListing;
use crate::shares::ConversationShare;
use crate::workflows::ConversationWorkflowList;
use crate::text::Trimmed;

/// The most characters (Unicode scalar values) a conversation's title has.
pub const TITLE_MAX: usize = 256;

/// The most items one batch changes.
pub const BATCH_MAX: usize = 100;

/// Where a conversation's work runs (`crates/backend-database/src/selection.rs`): the user's Cloud, a directory on a paired device, or a
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
/// [`ConversationPatch`] takes except `archived`. They apply as part of the
/// creation, and one that is refused refuses it.
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
    /// Its Workflow menu choice, which the new conversation was given before
    /// its first message; Default when absent.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "WorkflowChoice")]
    #[garde(dive)]
    pub workflow: Option<WorkflowChoice>,
}

/// What `POST /conversations` answers: the conversation as the list shows
/// it, and its metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreatedConversation {
    pub conversation: ConversationSummary,
    pub metadata: ConversationMetadata,
}

/// A conversation as the sidebar lists it: what the sidebar shows of it, and
/// no more. What a page shows only of an open conversation is its
/// [`ConversationMetadata`], and what the backend keeps for itself, such as
/// the owner, stays out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub id: ConversationId,
    /// Its title; null while it is the placeholder, which holds no words
    /// and which the page shows as New conversation in the reader's
    /// language.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub title: Option<String>,
    pub archived: bool,
    pub pinned: bool,
    /// The output revision the user last acknowledged.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub read_revision: u64,
    pub target: ConversationTarget,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub status: ConversationStatus,
    /// The output revision: it advances with each saved change of output,
    /// never with the user's input alone.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
    /// Whether output is newer than the read revision.
    pub unread: bool,
    /// How many permission requests of the conversation wait for the user,
    /// which the sidebar shows as the needs-you mark.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub permission_requests: u64,
    /// How many of those the user has not read: the sidebar's needs-you mark
    /// shows while this is above zero, and settles once the user reads the
    /// conversation, as it does for an answer. The read acknowledgement reads
    /// them with the output.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub unread_permission_requests: u64,
    /// The automation's run the conversation is filed under, which keeps it
    /// out of the sidebar until the user sends it a message or moves it; null
    /// for an ordinary conversation.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<crate::automations::FiledRun>")]
    #[garde(skip)]
    pub automation: Option<crate::automations::FiledRun>,
}

/// What a page shows of an open conversation beside its sidebar entry, its
/// agents and its transcripts: its settings, and each of its lists whole under
/// its revision (`docs/decisions/0029-a-conversation-opens-whole.md`). The
/// socket sends it before the open's handshake and again at each change, so
/// the page reads none of them by a route. Only a page that shows the
/// conversation hears of its changes, on the conversation's socket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMetadata {
    pub id: ConversationId,
    /// The conversation's model settings, the value every page shows; null
    /// while the conversation has no model yet.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<ModelSettings>")]
    pub model: Option<ModelSettings>,
    /// The conversation's Workflow menu choice
    /// (`crates/backend-user-shard/src/conversation/workflows.rs`), kept as its
    /// model is.
    pub workflow: WorkflowChoice,
    /// The directory the conversation's work runs in, resolved by the backend
    /// for every kind of target, so the web app never derives it.
    pub cwd: String,
    /// The root's latest ended turn; null before the first.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<LastTurn>")]
    pub last_turn: Option<LastTurn>,
    /// The speed of the root's last turn that streamed for a second or more,
    /// which the chips row shows once the turn's own reading has gone; null
    /// before the first (`docs/decisions/0012-live-output-tokens.md`).
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<TurnSpeed>")]
    pub turn_speed: Option<TurnSpeed>,
    /// Whether the conversation's tree is open with commands of plugins the
    /// user has since turned on or off, so a reload would change them
    /// (`crates/backend-plugins/src/lib.rs`).
    pub plugins_changed: bool,
    /// What the conversation's rows of no agent cost, such as its titles';
    /// each agent's own is its entry's, and the conversation's is their sum.
    #[garde(dive)]
    pub other_cost: AgentCost,
    /// The messages the conversation keeps saved for later, as the composer
    /// lists them, under the revision of the list.
    pub saved: SavedListing,
    /// The conversation's work panel: its tabs under the panel's revision, with
    /// the states the plugins hold for it.
    #[garde(dive)]
    pub panel: PanelRead,
    /// The conversation's attached hosts, under a revision an attach, a
    /// detach or a rename raises.
    pub hosts: ConversationHosts,
    /// The workflows of its working directory as the backend keeps them,
    /// under their revision, 0 before the backend first read them.
    pub workflows: ConversationWorkflowList,
    /// The number of the conversation's latest attachment, 3 for `a3`, 0
    /// before its first (`crates/backend-host-access/src/attachment_commands.rs`): a message
    /// that names a higher number names one still to come, whose description
    /// the socket sends once it is uploaded.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub last_attachment: u64,
    /// The revision of each plugin's conversation state, in registration
    /// order: a page reads a state only when its revision is newer than the one
    /// it holds, counted in memory.
    #[garde(dive)]
    pub plugin_revisions: Vec<PluginRevision>,
    /// The conversation's undecided permission requests, under a revision
    /// that rises with each change of them in this run of the backend.
    #[garde(dive)]
    pub permissions: ConversationPermissions,
    /// The conversation's shares, its own and its answers', so the header
    /// and an answer open the share dialog on the existing link.
    #[garde(skip)]
    pub shares: Vec<ConversationShare>,
}

/// `GET /conversations/:id`: one conversation as the sidebar lists it, its
/// metadata, and its agents at their version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDetail {
    pub conversation: ConversationSummary,
    pub metadata: ConversationMetadata,
    pub agents: Version,
    /// The root first, then the subagents by number, then the side
    /// questions as they were asked.
    pub entries: Vec<AgentEntry>,
}

/// What the conversation's socket carries of the conversation itself, beside
/// its agents' frames (`crates/backend-user-shard/src/conversation/socket.rs`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ConversationFrame {
    /// The conversation's metadata, at the open and at each change of it.
    Conversation { metadata: Box<ConversationMetadata> },
    /// The conversation's draft at a revision newer than the one the page
    /// named at its open, and at each change of it; the page that wrote a
    /// change knows its revision from its save's answer.
    Draft { draft: ConversationDraft },
    /// The attachments the agent `node` uploaded since the socket's last
    /// metadata, which a reply of the node may already name.
    Attachments {
        node: NodeId,
        attachments: Vec<ConversationAttachment>,
    },
}

/// The latest ended turn of a conversation's root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LastTurn {
    /// The block that ended the turn, so a turn that a Resume continues
    /// ends anew.
    pub id: BlockId,
    pub outcome: TurnOutcome,
}

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

/// A conversation's model settings (`crates/shared-types/src/catalog.rs`): the
/// provider entry and model, the thinking effort and the
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

/// Where a conversation stands: running or compacting while its live tree
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

/// `?before=|after=|around=&edge=` of
/// `GET /conversations/:id/agents/:node/transcript`. Queries are the
/// backend's alone and are not emitted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptQuery {
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

/// One page of an agent's transcript: blocks within the page budget, wherever
/// requests begin and end, in their light form, with the failure facts of its error blocks
/// (`crates/backend-user-shard/src/conversation/failure_facts.rs`).
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
    /// The versions of its blocks that have others; none in a share.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<BlockVersions>,
    /// The attachments its blocks name that the agent's replies show.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ConversationAttachment>,
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

/// `GET /conversations/:id/agents/:node/blocks/:blockId`: a block whole.
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

/// `GET /conversations/:id/commands/:commandId`: where the `shell` call that
/// started a command lies, which a report's title brings into view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandCall {
    pub command_id: CommandId,
    /// The agent that ran it.
    pub node: NodeId,
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
    /// The conversation's Workflow menu choice, from its next spawn on.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "WorkflowChoice")]
    #[garde(dive)]
    pub workflow: Option<WorkflowChoice>,
    /// A switch of the conversation's execution target.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
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
    ThinkingEffort,
    ServiceTierId,
    Workflow,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConversationUpdate {
    pub conversation: ConversationSummary,
    /// Its metadata as the patch left it, such as the model it set.
    pub metadata: ConversationMetadata,
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

/// The answer of a Fork: the new conversation as the list shows it, and its
/// metadata, whose model settings are the ones it inherited from its source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ForkAnswer {
    pub conversation: ConversationSummary,
    pub metadata: ConversationMetadata,
}

#[cfg(test)]
mod tests {
    use super::*;

    // The entries a provider keeps on a block never leave the backend
    // (`crates/provider-claude-code/src/session.rs`): a page whose
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
            vec![serde_json::json!({ "type": "assistant", "uuid": "kept" }).into()];
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
