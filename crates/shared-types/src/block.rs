//! Transcript blocks (`runtime.md` § Block types): a session's history, one
//! block per row of its checkpoint. Every block has an id, the time it was
//! written and the model selection current then.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::{
    AgentMessage, BlockId, CommandId, MAX_SAFE_INTEGER, ModelSelection, Nullable, ProviderErrorDiagnostics,
    QuotaWindowName, Timestamp, TokenUsage, ToolResultContentBlock, ToolView, TurnId, UserContentBlock,
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
    /// Command reports that arrived together; hidden from the user.
    Wakeup(WakeupBlock),
    /// A human steer of the running turn.
    Steer(SteerBlock),
    /// A message from another agent of the tree.
    AgentMessage(AgentMessageBlock),
    /// A turn continues after a cut: a resume, compaction inside a turn, or a
    /// model switch that landed inside a turn and compacted.
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
    /// The agent's account reached its limit and the agent went on with
    /// another (`providers.md` § Account rotation); the model never sees it.
    AccountSwitch(AccountSwitchBlock),
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
            Block::AccountSwitch($inner) => $body,
            Block::CompactionBoundary($inner) => $body,
            Block::CompactionMarker($inner) => $body,
        }
    };
}

impl Block {
    pub fn id(&self) -> &BlockId {
        on_block!(self, block => &block.id)
    }

    /// When the block was written.
    pub fn created_at(&self) -> Timestamp {
        on_block!(self, block => block.created_at)
    }

    /// The entries of the vendor's own record of the session kept on the
    /// block; none for a block the model receives nothing of.
    pub fn entries(&self) -> &[serde_json::Value] {
        match self {
            Block::User(block) => &block.entries,
            Block::Context(block) => &block.entries,
            Block::Wakeup(block) => &block.entries,
            Block::Steer(block) => &block.entries,
            Block::AgentMessage(block) => &block.entries,
            Block::Resume(block) => &block.entries,
            Block::Thinking(block) => &block.entries,
            Block::RedactedThinking(block) => &block.entries,
            Block::Text(block) => &block.entries,
            Block::ToolCall(block) => &block.entries,
            Block::CompactionBoundary(block) => &block.entries,
            Block::Abort(_)
            | Block::Response(_)
            | Block::Error(_)
            | Block::AccountSwitch(_)
            | Block::CompactionMarker(_) => {
                &[]
            }
        }
    }

    /// The same, to add to; none for a block that cannot hold entries.
    pub fn entries_mut(&mut self) -> Option<&mut Vec<serde_json::Value>> {
        match self {
            Block::User(block) => Some(&mut block.entries),
            Block::Context(block) => Some(&mut block.entries),
            Block::Wakeup(block) => Some(&mut block.entries),
            Block::Steer(block) => Some(&mut block.entries),
            Block::AgentMessage(block) => Some(&mut block.entries),
            Block::Resume(block) => Some(&mut block.entries),
            Block::Thinking(block) => Some(&mut block.entries),
            Block::RedactedThinking(block) => Some(&mut block.entries),
            Block::Text(block) => Some(&mut block.entries),
            Block::ToolCall(block) => Some(&mut block.entries),
            Block::CompactionBoundary(block) => Some(&mut block.entries),
            Block::Abort(_)
            | Block::Response(_)
            | Block::Error(_)
            | Block::AccountSwitch(_)
            | Block::CompactionMarker(_) => {
                None
            }
        }
    }

    /// The run of steps the block belongs to (`runtime.md` § Work groups);
    /// none for a block that is not a step.
    pub fn run(&self) -> Option<&BlockId> {
        match self {
            Block::Thinking(block) => block.run.as_ref(),
            Block::RedactedThinking(block) => block.run.as_ref(),
            Block::ToolCall(block) => block.run.as_ref(),
            Block::Wakeup(block) => block.run.as_ref(),
            Block::AgentMessage(block) => block.run.as_ref(),
            _ => None,
        }
    }

    /// Sets the block's run; a block that cannot be a step takes none.
    pub fn set_run(&mut self, run: Option<BlockId>) {
        match self {
            Block::Thinking(block) => block.run = run,
            Block::RedactedThinking(block) => block.run = run,
            Block::ToolCall(block) => block.run = run,
            Block::Wakeup(block) => block.run = run,
            Block::AgentMessage(block) => block.run = run,
            _ => {}
        }
    }

    /// Whether the block is an editable target: only a `user` block is.
    pub fn is_editable(&self) -> bool {
        matches!(self, Block::User(_))
    }
}

/// How a block a client receives is serialized: without the entries of the
/// vendor's own record, which never leave the backend (`claude-code.md`
/// § The session a process resumes). Use with `#[serde(with = ...)]`.
pub mod client_block {
    use std::borrow::Cow;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::Block;

    pub fn serialize<S: Serializer>(block: &Block, serializer: S) -> Result<S::Ok, S::Error> {
        sent(block).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Block, D::Error> {
        Block::deserialize(deserializer)
    }

    /// `block` as a client receives it.
    pub(super) fn sent(block: &Block) -> Cow<'_, Block> {
        if block.entries().is_empty() {
            return Cow::Borrowed(block);
        }
        let mut sent = block.clone();
        if let Some(entries) = sent.entries_mut() {
            entries.clear();
        }
        Cow::Owned(sent)
    }
}

/// The same for blocks a client receives together.
pub mod client_blocks {
    use serde::ser::SerializeSeq as _;
    use serde::{Deserialize, Deserializer, Serializer};

    use super::{Block, client_block::sent};

    pub fn serialize<S: Serializer>(blocks: &[Block], serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(blocks.len()))?;
        for block in blocks {
            sequence.serialize_element(&sent(block))?;
        }
        sequence.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Block>, D::Error> {
        Vec::<Block>::deserialize(deserializer)
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

/// A message the user submitted, with the runtime's text for the turn.
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
    /// The runtime's text for the turn, a subagent's identity, which the
    /// model receives before the content; null for the root.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub preamble: Option<String>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them (`claude-code.md` § The session
    /// a process resumes); omitted when there are none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<serde_json::Value>,
}

/// What one context source told the node before a request (`runtime.md`
/// § Context), which the model receives as a user message.
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
    /// The source that answered: `execution`, [`INSTRUCTIONS_SOURCE`],
    /// [`WORKING_DIRECTORY_SOURCE`], or a plugin's id.
    #[garde(length(chars, min = 1, max = 64))]
    pub source: String,
    #[garde(skip)]
    pub text: String,
    /// What an instructions block's text holds, in its order, which the
    /// page lists (`instructions.md` § What the card lists); empty for
    /// another source's block.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[garde(dive)]
    pub instructions: Vec<InstructionEntry>,
    /// The working directory a [`WORKING_DIRECTORY_SOURCE`] block names,
    /// the node's from this block on, which the page reads the node's
    /// relative paths against (`subagents.md` § Working directory); absent
    /// on another source's block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(inner(length(min = 1)))]
    pub cwd: Option<String>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them (`claude-code.md` § The session
    /// a process resumes); omitted when there are none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<serde_json::Value>,
}

/// The context source of the user's personal instructions and the
/// project's instruction files (`instructions.md`), whose block replay
/// sends whole.
pub const INSTRUCTIONS_SOURCE: &str = "instructions";

/// The context source of the conversation's execution context
/// (`runtime.md` § Context), which also names what still runs after a
/// compaction (`compaction.md` § What the model receives afterward); no
/// plugin may be named so.
pub const EXECUTION_SOURCE: &str = "execution";

/// The context source of a subagent's working directory (`subagents.md`
/// § Working directory): its block names the directory the child's `shell`
/// calls start in, before its first request, after each change and after
/// each compaction. No plugin may be named so: a plugin's id has no `_`.
pub const WORKING_DIRECTORY_SOURCE: &str = "working_directory";

/// The context source of the skills plugin, whose block is the skills
/// catalog (`skills.md` § What the model sees), the context card's Skills
/// row (`compaction.md` § What fills the context).
pub const SKILLS_SOURCE: &str = "skills";

/// One part of an instructions block's text (`instructions.md` § What the
/// card lists).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum InstructionEntry {
    /// The user's personal instructions.
    Personal,
    /// A project file, by its path on the Host.
    File {
        #[garde(skip)]
        path: String,
    },
    /// The instructions of the workflow the conversation chose
    /// (`subagents.md` § Workflows of a project): its name, and its file's
    /// path on the Host.
    Workflow {
        #[garde(skip)]
        name: String,
        #[garde(skip)]
        path: String,
    },
}

impl InstructionEntry {
    /// The entry's section of an instructions block, holding `text`
    /// (`instructions.md` § The block).
    pub fn section(&self, text: &str) -> String {
        let (opening, closing) = self.tags();
        format!("{opening}\n{text}\n{closing}")
    }

    fn tags(&self) -> (String, &'static str) {
        match self {
            Self::Personal => (
                "<personal_instructions>".to_owned(),
                "</personal_instructions>",
            ),
            Self::File { path } => (
                format!("<project_instructions path=\"{}\">", crate::xml_escaped(path)),
                "</project_instructions>",
            ),
            Self::Workflow { name, .. } => (
                format!("<workflow name=\"{}\">", crate::xml_escaped(name)),
                "</workflow>",
            ),
        }
    }
}

/// Command reports that arrived together (`runtime.md` § Command reports).
/// The model receives their text, one paragraph each, as a user message or
/// as a steer, as the placement says; the text is rendered from the reports
/// where it is replayed, never stored beside them. The user sees each report
/// as a row.
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
    /// The reports, in the order they arrived.
    #[garde(dive)]
    pub reports: Vec<CommandReport>,
    /// The run of steps the block belongs to (`runtime.md` § Work groups):
    /// the id of the run's first block, which the backend sets as it stores
    /// the block and never changes.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub run: Option<BlockId>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them (`claude-code.md` § The session
    /// a process resumes); omitted when there are none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<serde_json::Value>,
}

/// What a command a `shell` call left running tells its node
/// (`runtime.md` § Command reports): its progress, or its end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandReport {
    #[garde(skip)]
    pub command_id: CommandId,
    /// The title of the call that started the command, its `description`;
    /// empty when that call is not known.
    #[garde(skip)]
    pub title: String,
    #[garde(dive)]
    pub event: ReportEvent,
    /// What the command printed since the node's last look, as a `shell`
    /// result shows it: bounded, with the lines that name what it leaves
    /// out and how to read it; empty when it printed nothing new.
    #[garde(skip)]
    pub output: String,
    /// The media the job viewed, which an end carries as the `shell` result
    /// would have (`runtime.md` § What a result attaches): each medium by
    /// reference, or the part that took its place, then the text of the
    /// lines about them; none for a report of progress.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[garde(dive)]
    pub media: Vec<ToolResultContentBlock>,
}

/// What a report tells of its command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ReportEvent {
    /// It still runs, and reports every `interval_ms`.
    Running {
        #[garde(range(max = MAX_SAFE_INTEGER))]
        running_ms: u64,
        /// How long it has printed nothing.
        #[garde(range(max = MAX_SAFE_INTEGER))]
        idle_ms: u64,
        #[garde(skip)]
        interval_ms: u32,
    },
    /// It exited, with its status when its end was recorded.
    Ended {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[garde(skip)]
        exit_code: Option<i32>,
    },
    /// It was stopped, by whom when that is known.
    Stopped {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[garde(dive)]
        by: Option<StoppedBy>,
    },
    /// It was lost, for `reason` (`runtime.md` § Lost commands).
    Lost {
        #[garde(skip)]
        reason: String,
    },
}

/// Who stopped a command, when it was not the node that ran it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum StoppedBy {
    /// The user, from a page.
    User,
    /// Another agent of the conversation, by its number.
    Agent {
        #[garde(range(max = MAX_SAFE_INTEGER))]
        number: u64,
    },
}

/// How a command ended, as the conversation's record of its output keeps
/// it (`storage.md` § Command outputs).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CommandEnd {
    Exited {
        #[garde(skip)]
        exit_code: i32,
    },
    /// It was stopped: by a page, `demi shell stop`, a Stop of its action,
    /// or the end of its node's shells.
    Stopped,
    /// Its Host lost it, for this reason (`runtime.md` § Lost commands).
    Lost {
        #[garde(length(chars, min = 1, max = 1024))]
        reason: String,
    },
    /// Its record was made by a release before 0.1.21, which kept no end.
    Unrecorded,
}

/// Where command reports entered the transcript.
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
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them (`claude-code.md` § The session
    /// a process resumes); omitted when there are none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<serde_json::Value>,
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
    /// The run of steps the block belongs to (`runtime.md` § Work groups):
    /// the id of the run's first block, which the backend sets as it stores
    /// the block and never changes.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub run: Option<BlockId>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them (`claude-code.md` § The session
    /// a process resumes); omitted when there are none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<serde_json::Value>,
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
/// you left off.", or, when the system resumed it, why
/// (`failures-and-recovery.md` § The unfinished turn).
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
    /// Why the turn is resumed, as the model reads it before "Continue
    /// from where you left off.": "Demi resumed this turn after the backend
    /// restarted.", or, when the user resumes a turn its Host's absence left
    /// unfinished, "MacBook Pro is back online."; omitted when the user
    /// resumes with nothing to add.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub reason: Option<String>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them (`claude-code.md` § The session
    /// a process resumes); omitted when there are none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<serde_json::Value>,
}

/// The agent's account reached its limit and the same request went on with
/// another account: *alex@example.com reached its 5-hour limit, resetting
/// at 9:00 PM; continuing with sam@example.com*. The page words it, the
/// window's name in the viewer's language and the reset in their time zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSwitchBlock {
    #[garde(skip)]
    pub id: BlockId,
    #[garde(skip)]
    pub created_at: Timestamp,
    #[garde(dive)]
    pub model: ModelSelection,
    /// The label of the account that reached its limit.
    #[garde(skip)]
    pub from: String,
    /// The label of the account the agent continues with.
    #[garde(skip)]
    pub to: String,
    /// What names the window that was used up, such as a limit of five
    /// hours; null when the failure and the account's quota name none.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<QuotaWindowName>")]
    #[garde(dive)]
    pub window: Option<QuotaWindowName>,
    /// When the used-up account can be used again; null when unknown.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Timestamp>")]
    #[garde(skip)]
    pub resets_at: Option<Timestamp>,
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
    /// The run of steps the block belongs to (`runtime.md` § Work groups):
    /// the id of the run's first block, which the backend sets as it stores
    /// the block and never changes.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub run: Option<BlockId>,
    /// The length in characters (Unicode scalar values) of the text without
    /// the white space around it, which a page's light form carries in
    /// place of the text, so the page tells a withheld text from none
    /// (`web-api.md` § Light form); omitted from a block whole.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "u64")]
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub text_length: Option<u64>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them (`claude-code.md` § The session
    /// a process resumes); omitted when there are none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<serde_json::Value>,
}

/// Opaque reasoning data, replayed whole; a page shows it as thinking
/// without text while it is the newest block of a running turn.
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
    /// The run of steps the block belongs to (`runtime.md` § Work groups):
    /// the id of the run's first block, which the backend sets as it stores
    /// the block and never changes.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub run: Option<BlockId>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them (`claude-code.md` § The session
    /// a process resumes); omitted when there are none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<serde_json::Value>,
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
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them (`claude-code.md` § The session
    /// a process resumes); omitted when there are none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<serde_json::Value>,
}

/// What a stored `shell` call ran: its title and its script.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ShellCommand {
    /// The title the model wrote for the step, which the user sees.
    #[serde(rename = "description")]
    pub title: String,
    pub script: String,
}

impl ToolCallBlock {
    /// What the call ran, read from the input text it keeps, when it is a
    /// `shell` call that started a command or was suppressed as a repeat;
    /// none for any other call. The shell tool checked the input when the
    /// call was made (`runtime.md` § Tool input); this reads the two fields
    /// a stored call is shown and searched by
    /// (`compaction.md` § Reading the earlier history).
    pub fn shell_command(&self) -> Option<ShellCommand> {
        self.view.as_ref()?;
        // A view is written only for a call whose input decoded; an input
        // that does not decode now names no command to show.
        serde_json::from_str(&self.input).ok()
    }
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
    /// When the call returned, set as its result is stored; null while it
    /// executes. A run's time adds up its calls' times from `created_at` to
    /// this (`runtime.md` § Work groups).
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Timestamp>")]
    #[garde(skip)]
    pub ended_at: Option<Timestamp>,
    /// The run of steps the block belongs to (`runtime.md` § Work groups):
    /// the id of the run's first block, which the backend sets as it stores
    /// the block and never changes.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub run: Option<BlockId>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them (`claude-code.md` § The session
    /// a process resumes); omitted when there are none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<serde_json::Value>,
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

/// One completed provider request, with the usage of its last call to the
/// model, which anchors the context estimate; none when the provider
/// reported none, and then it anchors nothing.
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
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
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "ProviderErrorDiagnostics")]
    #[garde(dive)]
    pub diagnostics: Option<ProviderErrorDiagnostics>,
    /// Set when the failure ended no turn, as a `compact` action whose
    /// summary request failed: no turn is left unfinished, and the product
    /// offers no Resume for it (`failures-and-recovery.md` § Retries).
    /// Omitted otherwise.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[garde(skip)]
    pub outside_turn: bool,
    /// The device of a `host_offline` failure, which the turn's Resume
    /// waits for (`failures-and-recovery.md` § The unfinished turn);
    /// omitted for any other failure.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "ErrorDevice")]
    #[garde(dive)]
    pub device: Option<ErrorDevice>,
    /// The account the failing request used, when the entry's rotation
    /// gave the agent one of its own, whose quota a usage limit that names
    /// no moment reads (`failures-and-recovery.md` § The unfinished turn);
    /// omitted otherwise.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub account: Option<String>,
}

/// The code of the error block a turn ends with when a call of it failed
/// because its Host was offline and no later call ran there
/// (`failures-and-recovery.md` § The unfinished turn).
pub const HOST_OFFLINE: &str = "host_offline";

/// A device a failure names: its id, by which the page follows its state,
/// and its name as its record gave it then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ErrorDevice {
    #[garde(length(min = 1))]
    pub id: String,
    #[garde(length(min = 1))]
    pub name: String,
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
    /// The user's own messages the pass kept, oldest first, each as the
    /// content its user message replays (`compaction.md` § One pass).
    #[garde(dive)]
    pub kept: Vec<KeptMessage>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them (`claude-code.md` § The session
    /// a process resumes); omitted when there are none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<serde_json::Value>,
}

/// A message of the user's that a compaction kept beside its summary: a
/// `user` block's preamble and content, or a `steer` block's content, the
/// one that crossed the pass's budget cut to its text
/// (`compaction.md` § One pass).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeptMessage {
    #[garde(dive)]
    pub content: Vec<UserContentBlock>,
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
