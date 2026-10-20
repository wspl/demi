//! The transcript itself: the blocks of its window in order, and its version.
//!
//! The blocks change only through methods that record each change in the
//! journal. The window is the part of the path a live session holds: from a
//! position, `start`, to the path's end. Every index here is a position on the
//! whole path.

use std::{collections::BTreeMap, rc::Rc, sync::Arc};

use demi_conversation_socket_protocol::{TranscriptPatch, TranscriptVersion, Versions};
use demi_provider_common::{AccountSwitch, ToolCall};
use demi_shared_types::{AutomationMark, MessageComment, 
    AbortBlock, AccountSwitchBlock, AgentMessage, AgentMessageBlock, Block, BlockId, Clock, CompactionBoundaryBlock,
    CompactionMarkerBlock, ContextBlock, Entry, ErrorBlock, InstructionEntry, KeptMessage, ModelSelection, ProviderErrorDiagnostics,
    RedactedThinkingBlock, ResponseBlock, ResumeBlock, SteerBlock, TextBlock, ThinkingBlock,
    Timestamp, TokenUsage, ToolCallBlock, ToolCallStatus, ToolResultContentBlock, ToolView, TurnId,
    UserBlock, UserContentBlock, WakeupBlock, WakeupPlacement, CommandReport,
};
use demi_agent_store::changes::run_of;
use serde_json::Value;

use super::{
    PatchBatch, is_interruption,
    earlier::Earlier,
    journal::{DirtyRows, Journal, patch_index},
};
use crate::IdSource;

/// A tool call the provider requested that has no result yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCall {
    pub tool_use_id: String,
    pub tool_name: String,
    /// The call's input as the JSON text the provider supplied.
    pub input: String,
}

pub struct TranscriptLog {
    /// The position of the first block held.
    start: usize,
    /// The blocks from `start` to the path's end.
    blocks: Vec<Block>,
    /// What the session keeps of the blocks before `start`.
    earlier: Earlier,
    /// The versions at each place of the path that holds more than one, as
    /// the store reads them; a place past the last block holds those of
    /// the block that comes there next, as after Regenerate cut an answer.
    versions: BTreeMap<usize, Versions>,
    journal: Journal,
    revision: u64,
    /// New for every log, at creation and at every restore, so a version
    /// taken before a restart never matches after it.
    epoch: String,
    ids: Rc<dyn IdSource>,
    clock: Arc<dyn Clock>,
}

impl TranscriptLog {
    pub fn new(blocks: Vec<Block>, ids: Rc<dyn IdSource>, clock: Arc<dyn Clock>) -> Self {
        let epoch = ids.next_id();
        Self {
            start: 0,
            blocks,
            earlier: Earlier::default(),
            versions: BTreeMap::new(),
            journal: Journal::default(),
            revision: 0,
            epoch,
            ids,
            clock,
        }
    }

    /// The blocks held: those from [`start`](Self::start) on.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The log of a window that starts at the position `start` of its path,
    /// with what `earlier` keeps of the blocks before it.
    pub fn starting_at(mut self, start: usize, earlier: Earlier) -> Self {
        self.start = start;
        self.earlier = earlier;
        self
    }

    /// The position of the first block held.
    pub fn start(&self) -> usize {
        self.start
    }

    /// How many blocks the path holds.
    pub fn len(&self) -> usize {
        self.start + self.blocks.len()
    }

    /// Whether the path holds no block.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The block at `position`, when it is held.
    pub fn get(&self, position: usize) -> Option<&Block> {
        self.blocks.get(position.checked_sub(self.start)?)
    }

    /// The blocks held from `position` on; all of them for a position
    /// before the window.
    pub fn from(&self, position: usize) -> &[Block] {
        let at = position.saturating_sub(self.start).min(self.blocks.len());
        &self.blocks[at..]
    }

    /// What the session keeps of the blocks before the window.
    pub fn earlier(&self) -> &Earlier {
        &self.earlier
    }

    /// Where replay starts: the position of the last `compaction_boundary`
    /// held, or the window's start.
    pub fn replay_start(&self) -> usize {
        self.start + crate::replay_start(&self.blocks)
    }

    /// The failure that left the last turn unfinished, when its record is
    /// an `error` block.
    pub fn turn_failure(&self) -> Option<&ErrorBlock> {
        match crate::turn_record(&self.blocks) {
            Some(Block::Error(error)) => Some(error),
            Some(_) => None,
            None => self.earlier.record().flatten(),
        }
    }

    /// The turn of the last block of a turn's input: the last turn.
    pub fn last_turn(&self) -> Option<&TurnId> {
        crate::last_turn(&self.blocks).or(self.earlier.last_turn())
    }

    /// The latest message the user or an automation wrote, when there is
    /// one: the automation's run and when it came for a run's message.
    pub fn user_input(&self) -> Option<crate::UserInput> {
        self.blocks
            .iter()
            .rev()
            .find_map(crate::user_input)
            .or_else(|| self.earlier.user_input().cloned())
    }

    /// Holds `blocks` too, the path's from `start` up to the window, with
    /// the versions at their places and what `earlier` keeps of the blocks
    /// before them, as an edit of a message before the window reads them.
    pub fn extend_back(&mut self, start: usize, mut blocks: Vec<Block>, versions: BTreeMap<usize, Versions>, earlier: Earlier) {
        assert_eq!(start + blocks.len(), self.start, "the blocks reach the window");
        blocks.append(&mut self.blocks);
        self.blocks = blocks;
        self.start = start;
        self.earlier = earlier;
        self.versions.extend(versions);
    }

    /// Lets go of the blocks before `position`, keeping what `earlier`
    /// keeps of them. The caller has saved them.
    pub fn release_before(&mut self, position: usize) {
        let count = position.saturating_sub(self.start).min(self.blocks.len());
        if count == 0 {
            return;
        }
        let rest = self.blocks.split_off(count);
        let released = std::mem::replace(&mut self.blocks, rest);
        self.earlier.fold(&released);
        self.start += count;
        self.versions = self.versions.split_off(&self.start);
    }

    /// The log with the versions its stored path holds.
    pub fn with_versions(mut self, versions: BTreeMap<usize, Versions>) -> Self {
        self.versions = versions;
        self
    }

    /// The versions at each place of the path that holds more than one.
    pub fn versions(&self) -> &BTreeMap<usize, Versions> {
        &self.versions
    }

    pub fn version(&self) -> TranscriptVersion {
        TranscriptVersion {
            epoch: self.epoch.clone(),
            revision: self.revision,
        }
    }

    /// The block with this id, looked for from the end, where recent changes
    /// are.
    pub fn find(&self, id: &BlockId) -> Option<&Block> {
        self.position(id).and_then(|index| self.get(index))
    }

    /// The patches recorded since the last call, as one batch that advances
    /// the revision; none when nothing changed.
    pub fn take_patches(&mut self) -> Option<PatchBatch> {
        let batch = self.journal.take(self.revision + 1)?;
        self.revision = batch.revision;
        Some(batch)
    }

    pub fn push_user(
        &mut self,
        turn_id: TurnId,
        model: &ModelSelection,
        content: Vec<UserContentBlock>,
        comments: Vec<MessageComment>,
        preamble: Option<String>,
        automation: Option<AutomationMark>,
    ) {
        let (id, created_at) = self.stamp();
        self.append(Block::User(UserBlock {
            id,
            turn_id,
            created_at,
            model: model.clone(),
            content,
            comments,
            preamble,
            automation,
            entries: Vec::new(),
        }));
    }

    pub fn push_context(
        &mut self,
        turn_id: TurnId,
        model: &ModelSelection,
        source: String,
        text: String,
        instructions: Vec<InstructionEntry>,
        cwd: Option<String>,
    ) {
        let (id, created_at) = self.stamp();
        self.append(Block::Context(ContextBlock {
            id,
            turn_id,
            created_at,
            model: model.clone(),
            source,
            text,
            instructions,
            cwd,
            entries: Vec::new(),
        }));
    }

    /// A human steer, under the steer's id.
    pub fn push_steer(
        &mut self,
        id: BlockId,
        turn_id: TurnId,
        model: &ModelSelection,
        content: Vec<UserContentBlock>,
        comments: Vec<MessageComment>,
    ) {
        let created_at = self.clock.now();
        self.append(Block::Steer(SteerBlock {
            id,
            turn_id,
            created_at,
            model: model.clone(),
            content,
            comments,
            entries: Vec::new(),
        }));
    }

    /// Command reports that arrived together, in the order they arrived.
    pub fn push_wakeup(
        &mut self,
        id: BlockId,
        turn_id: TurnId,
        model: &ModelSelection,
        placement: WakeupPlacement,
        reports: Vec<CommandReport>,
    ) {
        let created_at = self.clock.now();
        self.append(Block::Wakeup(WakeupBlock {
            run: None,
            id,
            turn_id,
            created_at,
            model: model.clone(),
            placement,
            reports,
            entries: Vec::new(),
        }));
    }

    /// An agent message, under the message's id.
    pub fn push_agent_message(
        &mut self,
        turn_id: TurnId,
        model: &ModelSelection,
        message: AgentMessage,
    ) {
        let created_at = self.clock.now();
        self.append(Block::AgentMessage(AgentMessageBlock {
            run: None,
            id: message.id.clone(),
            turn_id,
            created_at,
            model: model.clone(),
            message,
            entries: Vec::new(),
        }));
    }

    /// The turn continues after a cut; `reason` says what made Demi resume
    /// it, none when the user did.
    pub fn push_resume(&mut self, turn_id: TurnId, model: &ModelSelection, reason: Option<String>) {
        let (id, created_at) = self.stamp();
        self.append(Block::Resume(ResumeBlock {
            id,
            turn_id,
            created_at,
            model: model.clone(),
            reason,
            entries: Vec::new(),
        }));
    }

    /// Marks the latest stopped marker as continued.
    pub fn mark_latest_abort_resumed(&mut self) {
        let Some(at) = self
            .blocks
            .iter()
            .rposition(|block| matches!(block, Block::Abort(_)))
        else {
            return;
        };
        if let Block::Abort(abort) = &mut self.blocks[at] {
            abort.is_resumed = true;
        }
        self.record_replace(self.start + at);
    }

    /// Replaces every block held with `blocks`, the path's from the window's
    /// start on, as a history rewrite publishes its retained history.
    ///
    /// Answers the batch that publishes it: a `truncate` at the first block
    /// that differs and an `add` for each block from there, never the blocks
    /// before it. The changes recorded before it are superseded, and the
    /// rewrite's own save wrote its rows, so the batch marks none.
    ///
    /// The rows the rewrite leaves stay stored on their branch:
    ///
    /// - An edit or Regenerate, `version`, begins a new version at the first
    ///   block that differs, the latest of its place.
    /// - The session's own rewrite stands in for what it replaced, so the place
    ///   keeps its versions.
    /// - The places after it hold none.
    pub fn replace_all(&mut self, blocks: Vec<Block>, version: bool) -> PatchBatch {
        let kept = self.start + kept(&self.blocks, &blocks);
        let mut versions = std::mem::take(&mut self.versions);
        versions.split_off(&(kept + 1));
        if version {
            let count = match versions.get(&kept) {
                Some(held) => held.count + 1,
                None if self.len() > kept => 2,
                None => 1,
            };
            versions.remove(&kept);
            if count > 1 {
                versions.insert(kept, Versions { index: count, count });
            }
        }
        self.replace_from(kept, blocks, versions)
    }

    /// Replaces the path from `position` on with what a switch of versions
    /// made current: `blocks` is the path from the window's start on, and
    /// `versions` the versions of its places from `position` on.
    pub fn switch(&mut self, position: usize, blocks: Vec<Block>, versions: BTreeMap<usize, Versions>) -> PatchBatch {
        let mut kept = std::mem::take(&mut self.versions);
        kept.split_off(&position);
        kept.extend(versions);
        self.replace_from(position, blocks, kept)
    }

    /// Replaces the path from `position` on with what a switch of versions made
    /// current from a place before the window.
    ///
    /// The window starts at `start` now, with `blocks` from there and the
    /// `versions` of their places, and `earlier` keeps what is before it.
    pub fn switch_window(
        &mut self,
        position: usize,
        start: usize,
        blocks: Vec<Block>,
        versions: BTreeMap<usize, Versions>,
        earlier: Earlier,
    ) -> PatchBatch {
        self.start = start;
        self.earlier = earlier;
        self.replace_from(position, blocks, versions)
    }

    /// Publishes `blocks`, the path's from the window's start, from `from`
    /// on as one batch, with `versions`.
    fn replace_from(&mut self, from: usize, blocks: Vec<Block>, versions: BTreeMap<usize, Versions>) -> PatchBatch {
        self.journal = Journal::default();
        self.revision += 1;
        self.versions = versions;
        let patches = std::iter::once(TranscriptPatch::Truncate {
            length: patch_index(from),
        })
        .chain(blocks[from - self.start..].iter().enumerate().map(|(offset, block)| {
            TranscriptPatch::Add {
                index: patch_index(from + offset),
                value: block.clone(),
                versions: self.versions.get(&(from + offset)).copied(),
            }
        }))
        .collect();
        self.blocks = blocks;
        PatchBatch {
            revision: self.revision,
            patches,
            rows: DirtyRows::default(),
        }
    }

    /// Inserts compaction's summary, with the user's messages it keeps, at
    /// `index`, where the kept history begins, and returns its id.
    pub fn insert_compaction_boundary(
        &mut self,
        index: usize,
        model: &ModelSelection,
        summary: String,
        summary_tokens: u64,
        kept: Vec<KeptMessage>,
    ) -> BlockId {
        let (id, created_at) = self.stamp();
        let block = Block::CompactionBoundary(CompactionBoundaryBlock {
            id: id.clone(),
            created_at,
            model: model.clone(),
            summary,
            summary_tokens,
            kept,
            entries: Vec::new(),
        });
        // The blocks it moves past the boundary keep their versions;
        // the boundary shows none.
        let moved = self.versions.split_off(&index);
        self.versions
            .extend(moved.into_iter().map(|(position, versions)| (position + 1, versions)));
        self.journal.add(index, &block, None);
        self.blocks.insert(index - self.start, block);
        id
    }

    /// Appends the marker of the boundary `boundary_id` with the estimate of
    /// what it summarized.
    pub fn push_compaction_marker(
        &mut self,
        model: &ModelSelection,
        boundary_id: BlockId,
        compacted_tokens: u64,
    ) {
        let (id, created_at) = self.stamp();
        self.append(Block::CompactionMarker(CompactionMarkerBlock {
            id,
            created_at,
            model: model.clone(),
            boundary_id,
            compacted_tokens,
        }));
    }

    /// The stopped marker a Stop leaves.
    pub fn push_abort(&mut self, model: &ModelSelection) {
        let (id, created_at) = self.stamp();
        self.append(Block::Abort(AbortBlock {
            id,
            created_at,
            model: model.clone(),
            is_resumed: false,
        }));
    }

    pub fn push_error(
        &mut self,
        model: &ModelSelection,
        message: String,
        code: Option<String>,
        diagnostics: Option<ProviderErrorDiagnostics>,
        outside_turn: bool,
        account: Option<String>,
    ) {
        let (id, created_at) = self.stamp();
        self.append(Block::Error(ErrorBlock {
            id,
            created_at,
            model: model.clone(),
            message,
            code,
            diagnostics,
            outside_turn,
            device: None,
            account,
        }));
    }

    /// Whether a `user` block of the turn `turn` exists: the turn began.
    pub fn has_user_turn(&self, turn: &TurnId) -> bool {
        self.earlier.has_user_turn(turn)
            || self
                .blocks
                .iter()
                .any(|block| matches!(block, Block::User(user) if &user.turn_id == turn))
    }

    /// Whether the path holds the tool call `id`.
    pub fn holds_call(&self, id: &BlockId) -> bool {
        self.earlier.holds_call(id) || self.position(id).is_some()
    }

    /// Whether the last block already says why its turn is unfinished.
    pub fn ends_with_interruption(&self) -> bool {
        self.blocks.last().is_some_and(is_interruption)
    }

    /// Opens a reasoning block with `text`, as a provider's thinking start
    /// does.
    pub fn open_thinking(&mut self, model: &ModelSelection, text: String) {
        let (id, created_at) = self.stamp();
        self.append(Block::Thinking(ThinkingBlock {
            run: None,
            id,
            created_at,
            model: model.clone(),
            text,
            signature: None,
            text_length: None,
            entries: Vec::new(),
        }));
    }

    /// Adds reasoning text to the open reasoning block, or opens one with
    /// it: a block is open until the vendor signed it.
    pub fn append_thinking(&mut self, model: &ModelSelection, text: &str) {
        if let Some(at) = self.blocks.len().checked_sub(1)
            && let Block::Thinking(open) = &mut self.blocks[at]
            && open.signature.is_none()
        {
            open.text.push_str(text);
            self.journal.append_text(self.start + at, text);
            return;
        }
        self.open_thinking(model, text.to_owned());
    }

    /// Signs the latest reasoning block. A signature with no reasoning block
    /// before it has nothing to sign and is dropped.
    pub fn sign_thinking(&mut self, signature: String) {
        let Some(at) = self
            .blocks
            .iter()
            .rposition(|block| matches!(block, Block::Thinking(_)))
        else {
            return;
        };
        if let Block::Thinking(thinking) = &mut self.blocks[at] {
            thinking.signature = Some(signature);
        }
        self.record_replace(self.start + at);
    }

    pub fn push_redacted_thinking(&mut self, model: &ModelSelection, data: String) {
        let (id, created_at) = self.stamp();
        self.append(Block::RedactedThinking(RedactedThinkingBlock {
            id,
            created_at,
            model: model.clone(),
            data,
            run: None,
            entries: Vec::new(),
        }));
    }

    /// Adds answer text to the open text block, or opens one with it: a text
    /// block is open until it is marked complete.
    pub fn append_text(&mut self, model: &ModelSelection, text: &str) {
        if let Some(at) = self.blocks.len().checked_sub(1)
            && let Block::Text(open) = &mut self.blocks[at]
            && !open.forkable
        {
            open.text.push_str(text);
            self.journal.append_text(self.start + at, text);
            return;
        }
        let (id, created_at) = self.stamp();
        self.append(Block::Text(TextBlock {
            id,
            created_at,
            model: model.clone(),
            text: text.to_owned(),
            forkable: false,
            entries: Vec::new(),
        }));
    }

    /// Marks the last block's text complete, so a Fork may end at it, and
    /// answers whether it did. Text that follows a call still executing
    /// cannot be a Fork's end, so it stays as it is.
    pub fn complete_tail_text(&mut self) -> bool {
        let Some(at) = self.blocks.len().checked_sub(1) else {
            return false;
        };
        let (earlier, tail) = self.blocks.split_at_mut(at);
        let Block::Text(text) = &mut tail[0] else {
            return false;
        };
        let executing = earlier.iter().any(
            |block| matches!(block, Block::ToolCall(call) if call.status == ToolCallStatus::Executing),
        );
        if text.forkable || executing {
            return false;
        }
        text.forkable = true;
        self.record_replace(self.start + at);
        true
    }

    /// A requested call, saved as executing until the session completes it.
    ///
    /// Its input is kept as text: a vendor that could not parse the input hands
    /// its text over as a JSON string, and a call without input has an empty
    /// object.
    pub fn push_tool_call(&mut self, model: &ModelSelection, call: ToolCall) {
        let input = match call.input {
            Value::String(text) => text,
            Value::Null => "{}".to_owned(),
            other => other.to_string(),
        };
        let (id, created_at) = self.stamp();
        self.append(Block::ToolCall(ToolCallBlock {
            run: None,
            id,
            created_at,
            model: model.clone(),
            tool_use_id: call.tool_use_id,
            tool_name: call.tool_name,
            input,
            status: ToolCallStatus::Executing,
            output: Vec::new(),
            view: None,
            ended_at: None,
            entries: Vec::new(),
        }));
    }

    /// Records that the agent's account reached its limit and the request
    /// went on with another (`crates/backend-providers/src/llm/rotation.rs`).
    pub fn push_account_switch(&mut self, model: &ModelSelection, switch: AccountSwitch) {
        let (id, created_at) = self.stamp();
        self.append(Block::AccountSwitch(AccountSwitchBlock {
            id,
            created_at,
            model: model.clone(),
            from: switch.from,
            to: switch.to,
            window: switch.window,
            resets_at: switch.resets_at,
        }));
    }

    pub fn push_response(&mut self, model: &ModelSelection, usage: Option<TokenUsage>) {
        let (id, created_at) = self.stamp();
        self.append(Block::Response(ResponseBlock {
            id,
            created_at,
            model: model.clone(),
            usage,
        }));
    }

    /// Every call still executing, in order.
    pub fn pending_tool_calls(&self) -> Vec<PendingCall> {
        self.blocks
            .iter()
            .filter_map(|block| match block {
                Block::ToolCall(call) if call.status == ToolCallStatus::Executing => {
                    Some(PendingCall {
                        tool_use_id: call.tool_use_id.clone(),
                        tool_name: call.tool_name.clone(),
                        input: call.input.clone(),
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// Completes the latest executing call with this id: a model may reuse a
    /// tool-use id across requests, and the latest is the one waiting.
    pub fn complete_tool_call(
        &mut self,
        tool_use_id: &str,
        output: Vec<ToolResultContentBlock>,
        is_error: bool,
        view: Option<ToolView>,
    ) {
        let Some(index) = self.executing_call(tool_use_id) else {
            return;
        };
        let ended_at = self.clock.now();
        if let Block::ToolCall(call) = &mut self.blocks[index - self.start] {
            call.status = if is_error {
                ToolCallStatus::Error
            } else {
                ToolCallStatus::Completed
            };
            call.output = output;
            call.view = view;
            call.ended_at = Some(ended_at);
        }
        self.record_replace(index);
    }

    /// Where the call `tool_use_id` that is still executing stands: the
    /// latest such call, as [`complete_tool_call`](Self::complete_tool_call)
    /// finds it.
    pub fn executing_call(&self, tool_use_id: &str) -> Option<usize> {
        let at = self.blocks.iter().rposition(|block| {
            matches!(block, Block::ToolCall(call)
                if call.status == ToolCallStatus::Executing && call.tool_use_id == tool_use_id)
        })?;
        Some(self.start + at)
    }

    /// Where the latest call `tool_use_id` that completed stands.
    pub fn completed_call(&self, tool_use_id: &str) -> Option<usize> {
        let at = self.blocks.iter().rposition(|block| {
            matches!(block, Block::ToolCall(call)
                if call.status != ToolCallStatus::Executing && call.tool_use_id == tool_use_id)
        })?;
        Some(self.start + at)
    }

    /// Where the block `id` stands among those held, looked for from the
    /// end.
    pub fn position(&self, id: &BlockId) -> Option<usize> {
        let at = self.blocks.iter().rposition(|block| block.id() == id)?;
        Some(self.start + at)
    }

    /// Gives the call `id` the view `view`; a block that is gone, as after
    /// an edit cut it, takes none.
    pub fn replace_view(&mut self, id: &BlockId, view: ToolView) {
        let Some(index) = self.position(id) else {
            return;
        };
        if let Block::ToolCall(call) = &mut self.blocks[index - self.start] {
            call.view = Some(view);
            self.record_replace(index);
        }
    }

    /// Adds `entries` after those the block `id` holds. A block that is
    /// gone, as after an edit cut it, or one that holds none, takes none.
    pub fn add_entries(&mut self, id: &BlockId, entries: Vec<Entry>) {
        let Some(index) = self.position(id) else {
            return;
        };
        let Some(held) = self.blocks[index - self.start].entries_mut() else {
            return;
        };
        held.extend(entries);
        self.record_replace(index);
    }

    /// A new block's id and time.
    fn stamp(&self) -> (BlockId, Timestamp) {
        let id = BlockId::try_from(self.ids.next_id())
            .expect("an id source never gives an empty identity");
        (id, self.clock.now())
    }

    /// Appends `block` with the run of steps it joins, which the blocks
    /// before it decide once.
    fn append(&mut self, mut block: Block) {
        block.set_run(run_of(&self.blocks, &block));
        let index = self.len();
        self.journal.add(index, &block, self.versions.get(&index).copied());
        self.blocks.push(block);
    }

    fn record_replace(&mut self, index: usize) {
        self.journal.replace_block(index, &self.blocks[index - self.start]);
    }
}

/// How many blocks `old` and `new` share from the start: the first
/// position a rewrite from `old` to `new` changes.
pub fn kept(old: &[Block], new: &[Block]) -> usize {
    old.iter().zip(new).take_while(|(old, new)| old == new).count()
}
