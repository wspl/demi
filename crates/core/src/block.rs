//! Transcript blocks (`runtime.md` § Block types): a session's history, one
//! block per row of its checkpoint. Every block has an id, the time it was
//! written and the model selection current then.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::{
    AgentMessage, BlockId, MAX_SAFE_INTEGER, ModelSelection, Nullable, ProviderErrorDiagnostics,
    Timestamp, TokenUsage, ToolResultContentBlock, ToolView, TurnId, UserContentBlock,
};

/// One block of a transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Block {
    /// A message the user submitted: the only block a user can edit.
    User(UserBlock),
    /// The conversation's execution context changed since the node last saw
    /// it, such as a target switch; hidden from the user.
    Context(ContextBlock),
    /// A yield wakeup fired; hidden from the user.
    Wakeup(WakeupBlock),
    /// A human steer of the running turn.
    Steer(SteerBlock),
    /// A message from another agent of the tree.
    AgentMessage(AgentMessageBlock),
    /// A turn continues after a cut: a resume, compaction inside a turn, or an
    /// immediate model switch that compacted.
    Resume(ResumeBlock),
    /// The stopped marker a Stop leaves.
    Abort(AbortBlock),
    Thinking(ThinkingBlock),
    RedactedThinking(RedactedThinkingBlock),
    Text(TextBlock),
    ToolCall(ToolCallBlock),
    /// The usage of one completed provider request.
    Response(ResponseBlock),
    /// A failed request or an interrupted turn.
    Error(ErrorBlock),
    /// Compaction's summary, where the kept history begins.
    CompactionBoundary(CompactionBoundaryBlock),
    /// Compaction's estimate of what it summarized, at the end.
    CompactionMarker(CompactionMarkerBlock),
}

/// Evaluates `$body` with `$inner` bound to the block's variant, whichever it
/// is; every variant has an id, a time and a model.
macro_rules! on_block {
    ($block:expr, $inner:ident => $body:expr) => {
        match $block {
            Block::User($inner) => $body,
            Block::Context($inner) => $body,
            Block::Wakeup($inner) => $body,
            Block::Steer($inner) => $body,
            Block::AgentMessage($inner) => $body,
            Block::Resume($inner) => $body,
            Block::Abort($inner) => $body,
            Block::Thinking($inner) => $body,
            Block::RedactedThinking($inner) => $body,
            Block::Text($inner) => $body,
            Block::ToolCall($inner) => $body,
            Block::Response($inner) => $body,
            Block::Error($inner) => $body,
            Block::CompactionBoundary($inner) => $body,
            Block::CompactionMarker($inner) => $body,
        }
    };
}

impl Block {
    pub fn id(&self) -> &BlockId {
        on_block!(self, block => &block.id)
    }

    pub fn created_at(&self) -> Timestamp {
        on_block!(self, block => block.created_at)
    }

    /// The model selection that was current when the block was written.
    pub fn model(&self) -> &ModelSelection {
        on_block!(self, block => &block.model)
    }

    /// Whether the block is an editable target: only a `user` block is.
    pub fn is_editable(&self) -> bool {
        matches!(self, Block::User(_))
    }
}

/// Written by hand because the derive's `Self::Context` would name the
/// `Context` variant: a block is valid when its variant is, and a failure's
/// path starts at the variant's fields.
impl garde::Validate for Block {
    type Context = ();

    fn validate_into(
        &self,
        context: &(),
        parent: &mut dyn FnMut() -> garde::Path,
        report: &mut garde::Report,
    ) {
        on_block!(self, block => block.validate_into(context, parent, report))
    }
}

/// A message the user submitted, with the harness's text for the turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserBlock {
    #[garde(skip)]
    pub id: BlockId,
    /// The message's id, which starts the turn.
    #[garde(skip)]
    pub turn_id: TurnId,
    #[garde(skip)]
    pub created_at: Timestamp,
    #[garde(dive)]
    pub model: ModelSelection,
    /// The content as submitted.
    #[garde(dive)]
    pub content: Vec<UserContentBlock>,
    /// The harness's text for the turn, which the model receives before the
    /// content; null when the harness adds none.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub preamble: Option<String>,
}

/// The execution context the node's next request runs in, which the model
/// receives as a user message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextBlock {
    #[garde(skip)]
    pub id: BlockId,
    #[garde(skip)]
    pub turn_id: TurnId,
    #[garde(skip)]
    pub created_at: Timestamp,
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(skip)]
    pub text: String,
}

/// A fired yield wakeup. The model receives the fixed wakeup text as a user
/// message or as a steer, as the placement says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WakeupBlock {
    #[garde(skip)]
    pub id: BlockId,
    #[garde(skip)]
    pub turn_id: TurnId,
    #[garde(skip)]
    pub created_at: Timestamp,
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(skip)]
    pub placement: WakeupPlacement,
}

/// Where a fired wakeup entered the transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WakeupPlacement {
    /// It started a continuation, and opens an input turn.
    NewTurn,
    /// It joined the running turn at a continuation boundary.
    Steer,
}

serde_plain::derive_display_from_serialize!(WakeupPlacement);
serde_plain::derive_fromstr_from_deserialize!(WakeupPlacement);

/// A human steer, written at a continuation boundary. Its id is the steer's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SteerBlock {
    #[garde(skip)]
    pub id: BlockId,
    #[garde(skip)]
    pub turn_id: TurnId,
    #[garde(skip)]
    pub created_at: Timestamp,
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(dive)]
    pub content: Vec<UserContentBlock>,
}

/// A message from another agent of the tree. Its id is the message's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentMessageBlock {
    #[garde(custom(is_message_id(&self.message.id)))]
    pub id: BlockId,
    #[garde(skip)]
    pub turn_id: TurnId,
    #[garde(skip)]
    pub created_at: Timestamp,
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(dive)]
    pub message: AgentMessage,
}

/// A receipt's block has its message's id.
fn is_message_id(message_id: &BlockId) -> impl FnOnce(&BlockId, &()) -> garde::Result + '_ {
    move |id, _| {
        if id != message_id {
            return Err(garde::Error::new(format!(
                "must be the message's id, {message_id}"
            )));
        }
        Ok(())
    }
}

/// The turn continues after a cut; the model receives "Continue from where
/// you left off."
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResumeBlock {
    #[garde(skip)]
    pub id: BlockId,
    #[garde(skip)]
    pub turn_id: TurnId,
    #[garde(skip)]
    pub created_at: Timestamp,
    #[garde(dive)]
    pub model: ModelSelection,
}

/// The stopped marker. The user sees it until the turn is continued.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AbortBlock {
    #[garde(skip)]
    pub id: BlockId,
    #[garde(skip)]
    pub created_at: Timestamp,
    #[garde(dive)]
    pub model: ModelSelection,
    /// Set once the stopped turn is continued.
    #[garde(skip)]
    pub is_resumed: bool,
}

/// Reasoning text, with the vendor's signature when it signed it. A signed
/// block is replayed whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThinkingBlock {
    #[garde(skip)]
    pub id: BlockId,
    #[garde(skip)]
    pub created_at: Timestamp,
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(skip)]
    pub text: String,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub signature: Option<String>,
}

/// Opaque reasoning data, replayed whole and never shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RedactedThinkingBlock {
    #[garde(skip)]
    pub id: BlockId,
    #[garde(skip)]
    pub created_at: Timestamp,
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(skip)]
    pub data: String,
}

/// Assistant text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextBlock {
    #[garde(skip)]
    pub id: BlockId,
    #[garde(skip)]
    pub created_at: Timestamp,
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(skip)]
    pub text: String,
    /// Set once the text is complete and a Fork may start after it
    /// (`conversation-fork.md`); omitted until then.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[garde(skip)]
    pub forkable: bool,
}

/// A tool call the provider requested, completed by the session with its
/// result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolCallBlock {
    #[garde(skip)]
    pub id: BlockId,
    #[garde(skip)]
    pub created_at: Timestamp,
    #[garde(dive)]
    pub model: ModelSelection,
    /// The provider's id of the call.
    #[garde(skip)]
    pub tool_use_id: String,
    #[garde(skip)]
    pub tool_name: String,
    /// The call's input as the JSON text the provider supplied, which need
    /// not be valid JSON.
    #[garde(skip)]
    pub input: String,
    #[garde(skip)]
    pub status: ToolCallStatus,
    /// The result, once the call completed.
    #[garde(dive)]
    pub output: Vec<ToolResultContentBlock>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<ToolView>")]
    #[garde(dive)]
    pub view: Option<ToolView>,
}

/// Where a tool call is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    /// Saved before the tool runs; a restore finds a call it never finished
    /// in this state.
    Executing,
    Completed,
    Error,
}

serde_plain::derive_display_from_serialize!(ToolCallStatus);
serde_plain::derive_fromstr_from_deserialize!(ToolCallStatus);

/// The usage of one completed provider request, which anchors the context
/// estimate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResponseBlock {
    #[garde(skip)]
    pub id: BlockId,
    #[garde(skip)]
    pub created_at: Timestamp,
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(dive)]
    pub usage: TokenUsage,
}

/// A failed request or an interrupted turn (`failures-and-recovery.md`
/// § The failure record).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ErrorBlock {
    #[garde(skip)]
    pub id: BlockId,
    #[garde(skip)]
    pub created_at: Timestamp,
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(skip)]
    pub message: String,
    /// The failure's code, such as `rate_limit` or `interrupted`; null when
    /// it has none.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub code: Option<String>,
    /// The provider's record of the failure, when a provider failed.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "ProviderErrorDiagnostics")]
    #[garde(dive)]
    pub diagnostics: Option<ProviderErrorDiagnostics>,
}

/// Compaction's summary of the history before it, inserted where the kept
/// history begins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompactionBoundaryBlock {
    #[garde(skip)]
    pub id: BlockId,
    #[garde(skip)]
    pub created_at: Timestamp,
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(skip)]
    pub summary: String,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub summary_tokens: u64,
}

/// Compaction's estimate of the size of what it summarized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompactionMarkerBlock {
    #[garde(skip)]
    pub id: BlockId,
    #[garde(skip)]
    pub created_at: Timestamp,
    #[garde(dive)]
    pub model: ModelSelection,
    /// The boundary this pass inserted.
    #[garde(skip)]
    pub boundary_id: BlockId,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub compacted_tokens: u64,
}
