//! What a session shows beside its transcript (`runtime.md` § Input): its
//! phase, the messages waiting to run, and the human steers it accepted but
//! has not yet written.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    BlockId, InstructionEntry, MAX_SAFE_INTEGER, ModelSelection, Nullable, TurnId, UserContentBlock,
};

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

/// How full the session's next request is, as compaction estimates it
/// (`compaction.md` § Context estimate), with the window its thresholds use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextUsage {
    /// The estimate of the next request, in tokens.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub tokens: u64,
    /// The window the thresholds use, in tokens; null for a model that
    /// reports none.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u64>")]
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub window: Option<u64>,
    /// The estimate from which a `compact` frame is taken; null when any is.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u64>")]
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub compact_from: Option<u64>,
    /// What fills the context (`compaction.md` § What fills the context),
    /// in the card's order, a row with no tokens left out. The rows before
    /// the compaction buffer add up to the estimate, or to more when the
    /// rows above Messages alone do.
    #[garde(dive)]
    pub rows: Vec<ContextRow>,
}

/// One row of what fills the context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextRow {
    #[garde(skip)]
    pub kind: ContextRowKind,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub tokens: u64,
    /// Whether the row is counted from text, which the card says with `~`;
    /// the compaction buffer and the free space are exact.
    #[garde(skip)]
    pub estimated: bool,
    /// What the row opens into; empty for a row that does not open.
    #[garde(dive)]
    pub parts: Vec<ContextPart>,
}

/// What a row of the context covers, in the card's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ContextRowKind {
    /// Everything else the request carries: the estimate less every row
    /// above, never below zero.
    Messages,
    /// The system prompt but its capability index, and the tools'
    /// definitions.
    SystemPrompt,
    /// The capability index, by command group.
    Commands,
    /// The instructions blocks replayed, by file.
    Instructions,
    /// The skills catalog's block, by skill.
    Skills,
    /// Every other context block replayed, by source.
    OtherContext,
    /// The replayed summary of the last compaction.
    Summary,
    /// The room the token threshold keeps: the window less the threshold.
    CompactionBuffer,
    /// What is left: the window less the rows above and the buffer.
    FreeSpace,
}

/// A part a row of the context opens into, with its estimate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ContextPart {
    /// A layer of the system prompt.
    Prompt {
        #[garde(skip)]
        layer: PromptLayerKind,
        #[garde(range(max = MAX_SAFE_INTEGER))]
        tokens: u64,
    },
    /// A tool's definition, such as the `shell` tool's.
    Tool {
        #[garde(skip)]
        name: String,
        #[garde(range(max = MAX_SAFE_INTEGER))]
        tokens: u64,
    },
    /// A command group, such as `demi browser`: its section of the
    /// capability index, or in Messages the calls that run it. Calls that
    /// run no `demi` group are the group `shell`.
    CommandGroup {
        #[garde(skip)]
        group: String,
        #[garde(range(max = MAX_SAFE_INTEGER))]
        tokens: u64,
    },
    /// The personal instructions or a project file of an instructions
    /// block.
    Instruction {
        #[garde(dive)]
        entry: InstructionEntry,
        #[garde(range(max = MAX_SAFE_INTEGER))]
        tokens: u64,
    },
    /// A skill of the catalog, by its name.
    Skill {
        #[garde(skip)]
        name: String,
        #[garde(range(max = MAX_SAFE_INTEGER))]
        tokens: u64,
    },
    /// The blocks of one context source, such as `execution`.
    Source {
        #[garde(skip)]
        source: String,
        #[garde(range(max = MAX_SAFE_INTEGER))]
        tokens: u64,
    },
    /// The user's messages and steers, without their media.
    UserMessages {
        #[garde(range(max = MAX_SAFE_INTEGER))]
        tokens: u64,
    },
    /// The agent's answer text.
    AgentText {
        #[garde(range(max = MAX_SAFE_INTEGER))]
        tokens: u64,
    },
    /// The agent's thinking.
    Thinking {
        #[garde(range(max = MAX_SAFE_INTEGER))]
        tokens: u64,
    },
    /// The images and documents the requests carry with their bytes.
    Images {
        #[garde(range(max = MAX_SAFE_INTEGER))]
        tokens: u64,
    },
    /// What Messages holds beyond its other parts: what the text estimates
    /// miss of the provider's count, and the blocks no other part names.
    Unattributed {
        #[garde(range(max = MAX_SAFE_INTEGER))]
        tokens: u64,
    },
}

impl ContextPart {
    /// The part's estimate, in tokens.
    pub fn tokens(&self) -> u64 {
        match self {
            Self::Prompt { tokens, .. }
            | Self::Tool { tokens, .. }
            | Self::CommandGroup { tokens, .. }
            | Self::Instruction { tokens, .. }
            | Self::Skill { tokens, .. }
            | Self::Source { tokens, .. }
            | Self::UserMessages { tokens }
            | Self::AgentText { tokens }
            | Self::Thinking { tokens }
            | Self::Images { tokens }
            | Self::Unattributed { tokens } => *tokens,
        }
    }
}

/// A layer of the system prompt (`system-prompt.md` § The layers), but the
/// capability index, which is a row of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PromptLayerKind {
    Identity,
    HarnessGuide,
    ToolRules,
    ModelIdentity,
    /// A subagent profile's added instructions.
    ProfileInstructions,
}

impl ContextUsage {
    /// Whether the user may compact now (`compaction.md` § When compaction
    /// runs).
    pub fn admits_compaction(&self) -> bool {
        self.compact_from.is_none_or(|from| self.tokens >= from)
    }
}

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

/// A call the model is writing (`runtime.md` § Calls being written): the
/// provider opened it and has not handed it over whole. It is never saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct PendingCall {
    /// The call's tool-use ID, which its `tool_call` block will have.
    #[garde(length(min = 1))]
    pub tool_use_id: String,
    #[garde(length(min = 1))]
    pub tool_name: String,
    /// The call's `description` once the model has written that string
    /// whole; null until then.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "crate::Nullable<String>")]
    #[garde(skip)]
    pub description: Option<String>,
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
