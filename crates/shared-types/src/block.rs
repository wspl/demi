//! Transcript blocks: a session's history, one block per row of its
//! checkpoint. Every block has an id, the time it was written and the model
//! selection current then.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::{
    AgentMessage, AutomationId, AutomationRunId, BlockId, CommandId, MAX_SAFE_INTEGER, MessageComment, ModelSelection, comments_content, Nullable, ProviderErrorDiagnostics,
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
    /// another (`crates/backend-providers/src/llm/rotation.rs`); the model never sees it.
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
    pub fn entries(&self) -> &[Entry] {
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
    pub fn entries_mut(&mut self) -> Option<&mut Vec<Entry>> {
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

    /// The run of steps the block belongs to; none for a block that is not a
    /// step.
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

    /// The request the block belongs to, by its `user` block's id; none for
    /// a block before the transcript's first `user` block.
    pub fn request(&self) -> Option<&BlockId> {
        on_block!(self, block => block.request.as_ref())
    }

    /// Sets the request the block belongs to.
    pub fn set_request(&mut self, request: Option<BlockId>) {
        on_block!(self, block => block.request = request)
    }

    /// Whether the block is an editable target: only a `user` block is.
    pub fn is_editable(&self) -> bool {
        matches!(self, Block::User(_))
    }
}

/// One entry of a vendor's own record of the session, kept on a block
/// (`crates/provider-claude-code/src/session.rs`), held as its JSON
/// text: a session replays a block's entries as they came and reads a
/// field of them seldom, so it holds the text, a third of the decoded
/// value's size, and decodes an entry only where a field of it is read.
#[derive(Debug, Clone)]
pub struct Entry(Box<serde_json::value::RawValue>);

impl Entry {
    /// The entry `value` is.
    pub fn new(value: &impl Serialize) -> serde_json::Result<Self> {
        serde_json::value::to_raw_value(value).map(Self)
    }

    /// The entry as its JSON text.
    pub fn json(&self) -> &str {
        self.0.get()
    }

    /// The entry decoded as `T`, such as only the fields a reader needs.
    pub fn decode<'a, T: Deserialize<'a>>(&'a self) -> serde_json::Result<T> {
        serde_json::from_str(self.0.get())
    }
}

impl From<serde_json::Value> for Entry {
    fn from(value: serde_json::Value) -> Self {
        Self::new(&value).expect("a JSON value serializes")
    }
}

/// Two entries are one when their texts are: an entry is always written
/// by the same serializer, compact and in its own key order.
impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.json() == other.json()
    }
}

impl Eq for Entry {}

impl Serialize for Entry {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

/// A block is a tagged enum, which serde reads through its own buffer
/// rather than serde_json's reader, and serde_json reads a raw value only
/// from its own: an entry is read as a value and written back as text.
impl<'de> Deserialize<'de> for Entry {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Self::new(&value).map_err(serde::de::Error::custom)
    }
}

/// How a block a client receives is serialized: without the entries of the
/// vendor's own record, which never leave the backend
/// (`crates/provider-claude-code/src/session.rs`). Use with `#[serde(with =
/// ...)]`.
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
    /// The request the block belongs to: the id of the `user` block that
    /// starts it, the block's own for a `user` block, which the backend sets
    /// as it stores the block and never changes; absent for a block before
    /// the transcript's first `user` block (`crates/agent-store/src/changes.rs`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub request: Option<BlockId>,
    #[garde(dive)]
    pub model: ModelSelection,
    /// The content as submitted.
    #[garde(dive)]
    pub content: Vec<UserContentBlock>,
    /// The comments sent with the message, in the order they were saved,
    /// which the model reads before the content; omitted when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[garde(dive)]
    pub comments: Vec<MessageComment>,
    /// The runtime's text for the turn, a subagent's identity, which the
    /// model receives before the content; null for the root.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub preamble: Option<String>,
    /// The automation's run that sent the message, which the transcript
    /// names above it as a receipt names the agent that sent one; absent
    /// for a message the user wrote.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "AutomationMark")]
    #[garde(dive)]
    pub automation: Option<AutomationMark>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them
    /// (`crates/provider-claude-code/src/session.rs`); omitted when there are
    /// none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<Entry>,
}

impl UserBlock {
    /// What the model reads before the message's content, in order: the
    /// preamble, then the comments sent with it, a picture's as its
    /// image.
    pub fn lead_content(&self) -> Vec<UserContentBlock> {
        self.preamble
            .iter()
            .map(|text| UserContentBlock::Text { text: text.clone() })
            .chain(comments_content(&self.comments))
            .collect()
    }
}

/// The automation's run a message came from: which automation, as it was named
/// when it fired, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutomationMark {
    #[garde(skip)]
    pub automation_id: AutomationId,
    #[garde(skip)]
    pub run_id: AutomationRunId,
    /// The automation's name when the run started; the automation may be
    /// renamed or deleted since.
    #[garde(length(chars, min = 1))]
    pub name: String,
    /// When its trigger fired.
    #[garde(skip)]
    pub at: Timestamp,
}

/// What one context source told the node before a request, which the model
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
    /// The request the block belongs to: the id of the `user` block that
    /// starts it, the block's own for a `user` block, which the backend sets
    /// as it stores the block and never changes; absent for a block before
    /// the transcript's first `user` block (`crates/agent-store/src/changes.rs`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub request: Option<BlockId>,
    #[garde(dive)]
    pub model: ModelSelection,
    /// The source that answered: `execution`, [`INSTRUCTIONS_SOURCE`],
    /// [`WORKING_DIRECTORY_SOURCE`], [`REPORTING_SOURCE`], or a plugin's id.
    #[garde(length(chars, min = 1, max = 64))]
    pub source: String,
    #[garde(skip)]
    pub text: String,
    /// What an instructions block's text holds, in its order, which the
    /// page lists; empty for another source's block.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[garde(dive)]
    pub instructions: Vec<InstructionEntry>,
    /// The working directory a [`WORKING_DIRECTORY_SOURCE`] block names,
    /// the node's from this block on, which the page reads the node's
    /// relative paths against (`crates/agent-server/src/node/mod.rs`); absent
    /// on another source's block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(inner(length(min = 1)))]
    pub cwd: Option<String>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them
    /// (`crates/provider-claude-code/src/session.rs`); omitted when there are
    /// none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<Entry>,
}

/// The context source of the user's personal instructions and the
/// project's instruction files, whose block replay sends whole.
pub const INSTRUCTIONS_SOURCE: &str = "instructions";

/// The context source of what the user asked aside, in side questions.
pub const SIDE_QUESTIONS_SOURCE: &str = "side_questions";

/// The context source of the conversation's execution context, which
/// also names what still runs after a compaction; no plugin may be named
/// so.
pub const EXECUTION_SOURCE: &str = "execution";

/// The context source of a subagent's working directory
/// (`crates/agent-server/src/node/mod.rs`): its block names the directory the
/// child's `shell` calls start in, before its first request, after each change
/// and after each compaction. No plugin may be named so: a plugin's id has no
/// `_`.
pub const WORKING_DIRECTORY_SOURCE: &str = "working_directory";

/// The context source of what a subagent is told of how its parent hears from
/// it (`crates/agent-server/src/node/mod.rs`): before its first request and
/// after each compaction. No plugin may be named so: a plugin's id has no `_`.
pub const REPORTING_SOURCE: &str = "agent_reporting";

/// The context source of the skills plugin, whose block is the skills
/// catalog, the context card's Skills row.
pub const SKILLS_SOURCE: &str = "skills";

/// One part of an instructions block's text.
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
    /// (`crates/backend-user-shard/src/conversation/workflows.rs`): its name, and its file's
    /// path on the Host.
    Workflow {
        #[garde(skip)]
        name: String,
        #[garde(skip)]
        path: String,
    },
}

impl InstructionEntry {
    /// The entry's section of an instructions block, holding `text`.
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

/// Command reports that arrived together (`crates/agent-session/src/reports.rs`).
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
    /// The request the block belongs to: the id of the `user` block that
    /// starts it, the block's own for a `user` block, which the backend sets
    /// as it stores the block and never changes; absent for a block before
    /// the transcript's first `user` block (`crates/agent-store/src/changes.rs`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub request: Option<BlockId>,
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(skip)]
    pub placement: WakeupPlacement,
    /// The reports, in the order they arrived.
    #[garde(dive)]
    pub reports: Vec<CommandReport>,
    /// The run of steps the block belongs to: the id of the run's first
    /// block, which the backend sets as it stores the block and never
    /// changes.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub run: Option<BlockId>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them
    /// (`crates/provider-claude-code/src/session.rs`); omitted when there are
    /// none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<Entry>,
}

/// What a command a `shell` call left running tells its node
/// (`crates/agent-session/src/reports.rs`): its progress, or its end.
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
    /// would have (`crates/agent-tools/src/result.rs`): each medium by
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
    /// It was lost, for `reason` (`crates/agent-session/src/reports.rs`).
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
/// it (`crates/backend-database/src/command_outputs.rs`).
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
    /// Its Host lost it, for this reason (`crates/agent-session/src/reports.rs`).
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
    /// The request the block belongs to: the id of the `user` block that
    /// starts it, the block's own for a `user` block, which the backend sets
    /// as it stores the block and never changes; absent for a block before
    /// the transcript's first `user` block (`crates/agent-store/src/changes.rs`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub request: Option<BlockId>,
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(dive)]
    pub content: Vec<UserContentBlock>,
    /// The comments sent with it, which the model reads before its content
    /// as a message's; omitted when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[garde(dive)]
    pub comments: Vec<MessageComment>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them
    /// (`crates/provider-claude-code/src/session.rs`); omitted when there are
    /// none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<Entry>,
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
    /// The request the block belongs to: the id of the `user` block that
    /// starts it, the block's own for a `user` block, which the backend sets
    /// as it stores the block and never changes; absent for a block before
    /// the transcript's first `user` block (`crates/agent-store/src/changes.rs`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub request: Option<BlockId>,
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(dive)]
    pub message: AgentMessage,
    /// The run of steps the block belongs to: the id of the run's first
    /// block, which the backend sets as it stores the block and never
    /// changes.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub run: Option<BlockId>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them
    /// (`crates/provider-claude-code/src/session.rs`); omitted when there are
    /// none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<Entry>,
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
/// you left off.", or, when the system resumed it, why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResumeBlock {
    #[garde(skip)]
    pub id: BlockId,
    #[garde(skip)]
    pub turn_id: TurnId,
    #[garde(skip)]
    pub created_at: Timestamp,
    /// The request the block belongs to: the id of the `user` block that
    /// starts it, the block's own for a `user` block, which the backend sets
    /// as it stores the block and never changes; absent for a block before
    /// the transcript's first `user` block (`crates/agent-store/src/changes.rs`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub request: Option<BlockId>,
    #[garde(dive)]
    pub model: ModelSelection,
    /// Why the turn is resumed, as the model reads it before "Continue
    /// from where you left off.": "Demi resumed this turn after the backend
    /// restarted."; omitted when the user resumes with nothing to add.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub reason: Option<String>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them
    /// (`crates/provider-claude-code/src/session.rs`); omitted when there are
    /// none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<Entry>,
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
    /// The request the block belongs to: the id of the `user` block that
    /// starts it, the block's own for a `user` block, which the backend sets
    /// as it stores the block and never changes; absent for a block before
    /// the transcript's first `user` block (`crates/agent-store/src/changes.rs`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub request: Option<BlockId>,
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
    /// The request the block belongs to: the id of the `user` block that
    /// starts it, the block's own for a `user` block, which the backend sets
    /// as it stores the block and never changes; absent for a block before
    /// the transcript's first `user` block (`crates/agent-store/src/changes.rs`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub request: Option<BlockId>,
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
    /// The request the block belongs to: the id of the `user` block that
    /// starts it, the block's own for a `user` block, which the backend sets
    /// as it stores the block and never changes; absent for a block before
    /// the transcript's first `user` block (`crates/agent-store/src/changes.rs`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub request: Option<BlockId>,
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(skip)]
    pub text: String,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub signature: Option<String>,
    /// The run of steps the block belongs to: the id of the run's first
    /// block, which the backend sets as it stores the block and never
    /// changes.
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
    /// place of the text, so the page tells a withheld text from none; omitted
    /// from a block whole.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "u64")]
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub text_length: Option<u64>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them
    /// (`crates/provider-claude-code/src/session.rs`); omitted when there are
    /// none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<Entry>,
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
    /// The request the block belongs to: the id of the `user` block that
    /// starts it, the block's own for a `user` block, which the backend sets
    /// as it stores the block and never changes; absent for a block before
    /// the transcript's first `user` block (`crates/agent-store/src/changes.rs`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub request: Option<BlockId>,
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(skip)]
    pub data: String,
    /// The run of steps the block belongs to: the id of the run's first
    /// block, which the backend sets as it stores the block and never
    /// changes.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub run: Option<BlockId>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them
    /// (`crates/provider-claude-code/src/session.rs`); omitted when there are
    /// none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<Entry>,
}

/// Assistant text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextBlock {
    #[garde(skip)]
    pub id: BlockId,
    #[garde(skip)]
    pub created_at: Timestamp,
    /// The request the block belongs to: the id of the `user` block that
    /// starts it, the block's own for a `user` block, which the backend sets
    /// as it stores the block and never changes; absent for a block before
    /// the transcript's first `user` block (`crates/agent-store/src/changes.rs`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub request: Option<BlockId>,
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(skip)]
    pub text: String,
    /// Set once the text is complete and a Fork may start after it;
    /// omitted until then. The transcript alone decides it: the text is
    /// complete when the provider's answer moves on to anything but more of
    /// it, or when the response ends. Text a failed request cut off stays
    /// unmarked, and so does text that follows a tool call still executing.
    /// The mark is part of the block, so it is saved and restored with it, and
    /// the page offers Fork only on a marked block.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[garde(skip)]
    pub forkable: bool,
    /// The inline code of the text that names a file on the conversation's
    /// Host, which opens as a link: read once through the Host when the text
    /// is complete (`crates/agent-server/src/node/reply_files.rs`) and kept
    /// with the block. Absent until then; empty when no code names a file, or
    /// when the Host could not say.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "Vec<CodeFile>")]
    #[garde(dive)]
    pub code_files: Option<Vec<CodeFile>>,
    /// The pictures of the Host the text shows, by the path it wrote, and what
    /// became of the copy the backend makes of each when the text is complete
    /// (`crates/agent-server/src/node/reply_files.rs`). Absent when no copy
    /// was asked for: a text that names no such picture, one that is not
    /// complete, and one written before copies were kept.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "Vec<ReplyPicture>")]
    #[garde(dive)]
    pub pictures: Option<Vec<ReplyPicture>>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them
    /// (`crates/provider-claude-code/src/session.rs`); omitted when there are
    /// none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<Entry>,
}

/// Inline code of a reply that names a file on the Host: the code as the
/// reply wrote it, such as `src/app.ts:42`, the file's absolute path and the
/// line it names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeFile {
    #[garde(length(min = 1))]
    pub code: String,
    #[garde(length(min = 1))]
    pub path: String,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u64>")]
    #[garde(range(min = 1, max = MAX_SAFE_INTEGER))]
    pub line: Option<u64>,
}

/// A picture of the Host a reply shows, and its copy: the path as the reply
/// wrote it, such as `out/chart.png`, and what the backend made of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplyPicture {
    #[garde(length(min = 1))]
    pub target: String,
    #[garde(dive)]
    pub copy: PictureCopy,
}

/// What became of the copy of a picture of the Host that a reply shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "state", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum PictureCopy {
    /// The copy is being made; the picture shows as a placeholder.
    Copying,
    /// The conversation's attachment that holds the copy, such as `a3`.
    Copied {
        #[garde(length(min = 1))]
        attachment: String,
    },
    /// The file could not be copied, and why; it is not read again.
    Failed {
        #[garde(dive)]
        reason: PictureFailure,
    },
}

/// Why a picture of the Host that a reply shows could not be copied. The
/// backend names the cause and the page words it in the user's language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum PictureFailure {
    /// The Host has no such file.
    Missing,
    /// The file is over the limit of an attachment; `size` is its bytes.
    TooLarge {
        #[garde(range(max = crate::MAX_SAFE_INTEGER))]
        size: u64,
    },
    /// The file is no image; `media_type` is what it is.
    NotAnImage {
        #[garde(length(min = 1))]
        media_type: String,
    },
    /// The Host could not be reached for the copy: it is offline or stopped,
    /// the conversation works elsewhere now, or it has no Host yet.
    HostUnavailable,
    /// The Host could not read the file, or the copy could not be kept.
    Unreadable,
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
    /// call was made (`crates/agent-tools/src/lib.rs`); this reads the two fields
    /// a stored call is shown and searched by.
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
    /// The request the block belongs to: the id of the `user` block that
    /// starts it, the block's own for a `user` block, which the backend sets
    /// as it stores the block and never changes; absent for a block before
    /// the transcript's first `user` block (`crates/agent-store/src/changes.rs`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub request: Option<BlockId>,
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
    /// this.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Timestamp>")]
    #[garde(skip)]
    pub ended_at: Option<Timestamp>,
    /// The run of steps the block belongs to: the id of the run's first
    /// block, which the backend sets as it stores the block and never
    /// changes.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub run: Option<BlockId>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them
    /// (`crates/provider-claude-code/src/session.rs`); omitted when there are
    /// none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<Entry>,
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
    /// The request the block belongs to: the id of the `user` block that
    /// starts it, the block's own for a `user` block, which the backend sets
    /// as it stores the block and never changes; absent for a block before
    /// the transcript's first `user` block (`crates/agent-store/src/changes.rs`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub request: Option<BlockId>,
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(dive)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

/// A failed request or an interrupted turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ErrorBlock {
    #[garde(skip)]
    pub id: BlockId,
    #[garde(skip)]
    pub created_at: Timestamp,
    /// The request the block belongs to: the id of the `user` block that
    /// starts it, the block's own for a `user` block, which the backend sets
    /// as it stores the block and never changes; absent for a block before
    /// the transcript's first `user` block (`crates/agent-store/src/changes.rs`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub request: Option<BlockId>,
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
    /// offers no Resume for it. Omitted otherwise.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[garde(skip)]
    pub outside_turn: bool,
    /// The device of a `host_offline` failure, which earlier releases
    /// recorded when a call found the turn's Host offline; nothing writes
    /// it now, and a stored one is read so that its block still decodes.
    /// Omitted otherwise.
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
    /// no moment reads; omitted otherwise.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub account: Option<String>,
}

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
    /// The request the block belongs to: the id of the `user` block that
    /// starts it, the block's own for a `user` block, which the backend sets
    /// as it stores the block and never changes; absent for a block before
    /// the transcript's first `user` block (`crates/agent-store/src/changes.rs`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub request: Option<BlockId>,
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(skip)]
    pub summary: String,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub summary_tokens: u64,
    /// The user's own messages the pass kept, oldest first.
    #[garde(dive)]
    pub kept: Vec<crate::KeptMessage>,
    /// The entries of the vendor's own record of the session that belong to
    /// this block, as its provider gave them
    /// (`crates/provider-claude-code/src/session.rs`); omitted when there are
    /// none. They never leave
    /// the backend: a block a client receives is serialized without them
    /// ([`client_block`]), so the web contract leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(skip)]
    #[garde(skip)]
    pub entries: Vec<Entry>,
}

/// Compaction's estimate of the size of what it summarized. It is appended at
/// the end, while its boundary sits where the kept history begins, so that
/// the page shows the divider where the compaction was triggered, not where
/// the summary sits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompactionMarkerBlock {
    #[garde(skip)]
    pub id: BlockId,
    #[garde(skip)]
    pub created_at: Timestamp,
    /// The request the block belongs to: the id of the `user` block that
    /// starts it, the block's own for a `user` block, which the backend sets
    /// as it stores the block and never changes; absent for a block before
    /// the transcript's first `user` block (`crates/agent-store/src/changes.rs`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "BlockId")]
    #[garde(skip)]
    pub request: Option<BlockId>,
    #[garde(dive)]
    pub model: ModelSelection,
    /// The boundary this pass inserted.
    #[garde(skip)]
    pub boundary_id: BlockId,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub compacted_tokens: u64,
}
