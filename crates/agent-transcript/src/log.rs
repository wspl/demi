//! The transcript itself: the blocks of its window in order, and its version.
//!
//! The blocks change only through methods that record each change in the
//! journal. The window is the part of the path a live session holds: from a
//! position, `start`, to the path's end. Every index here is a position on the
//! whole path.

use std::{collections::BTreeMap, rc::Rc, sync::Arc};

use demi_conversation_lane_protocol::{TranscriptPatch, Version, Versions};
use demi_provider_common::{AccountSwitch, OpaqueStep, ToolCall};
use demi_shared_types::{ScheduleMark, CodeFile, MessageComment, PictureCopy, ReplyPicture, 
    AbortBlock, AccountSwitchBlock, AgentMessage, AgentMessageBlock, Block, BlockId, Clock, CompactionBoundaryBlock,
    CompactionMarkerBlock, ContextBlock, Entry, ErrorBlock, InstructionEntry, KeptMessage, ModelSelection, ProviderErrorDiagnostics,
    OpaqueBlock, OpaqueStatus, ResponseBlock, ResumeBlock, SteerBlock, TextBlock, ThinkingBlock,
    Timestamp, TokenUsage, ToolCallBlock, ToolCallStatus, ToolResultContentBlock, ToolView, TurnId,
    UserBlock, UserContentBlock, WakeupBlock, WakeupPlacement, CommandReport,
};
use demi_agent_store::changes::{request_of, run_of};
use serde_json::Value;

use super::{
    PatchBatch, is_interruption, reply_markdown::named_pictures,
    cut::{ended_turn_answer, ends_turn, from_another_conversation, turn_of},
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

/// The end of a turn the user asked for, as it ended: the block that ended
/// it, whether it failed, and the Markdown of its last text that is not
/// blank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskedEnd {
    pub id: BlockId,
    pub failed: bool,
    pub answer: Option<String>,
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
    /// The turn the latest input opened or joined, and whether the user
    /// asked for it ([`TranscriptLog::take_asked_end`]), decided as its
    /// inputs arrive; none before an input arrived in this log.
    asking: Option<(TurnId, bool)>,
    /// The latest end of a turn the user asked for, until it is taken.
    asked_end: Option<AskedEnd>,
}

/// When a request left Demi and its first output came, which its response
/// block keeps.
#[derive(Debug, Clone, Copy)]
pub struct RequestTimes {
    pub sent_at: Timestamp,
    /// When its first attempt left, for a request sent again after the
    /// attempts before failed.
    pub first_sent_at: Option<Timestamp>,
    pub first_output_at: Option<Timestamp>,
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
            asking: None,
            asked_end: None,
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

    /// The latest message the user or a schedule wrote, when there is
    /// one: the schedule's run and when it came for a run's message.
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

    pub fn version(&self) -> Version {
        Version {
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
        schedule: Option<ScheduleMark>,
    ) {
        let (id, created_at) = self.stamp();
        self.append(Block::User(UserBlock {
            id,
            turn_id,
            created_at,
            request: None,
            model: model.clone(),
            content,
            comments,
            preamble,
            schedule,
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
            request: None,
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
            request: None,
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
            request: None,
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
        sender_below: bool,
    ) {
        let created_at = self.clock.now();
        self.append_received(Block::AgentMessage(AgentMessageBlock {
            run: None,
            id: message.id.clone(),
            turn_id,
            created_at,
            request: None,
            model: model.clone(),
            message,
            entries: Vec::new(),
        }), sender_below);
    }

    /// The turn continues after a cut; `reason` says what made Demi resume
    /// it, none when the user did.
    pub fn push_resume(&mut self, turn_id: TurnId, model: &ModelSelection, reason: Option<String>) {
        let (id, created_at) = self.stamp();
        self.append(Block::Resume(ResumeBlock {
            id,
            turn_id,
            created_at,
            request: None,
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
        // A rewritten path, as an edit or a switch of versions makes, says
        // anew whose its last turn is; an end kept before it may be gone.
        self.asking = self.held_turn();
        self.asked_end = None;
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
        // It continues the request of the block before it; at the window's
        // start, the request the block after it continues.
        let offset = index - self.start;
        let request = match offset.checked_sub(1).and_then(|before| self.blocks.get(before)) {
            Some(before) => before.request().cloned(),
            None => self
                .blocks
                .get(offset)
                .filter(|after| !matches!(after, Block::User(_)))
                .and_then(|after| after.request().cloned()),
        };
        let block = Block::CompactionBoundary(CompactionBoundaryBlock {
            id: id.clone(),
            created_at,
            request,
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
    /// what it summarized, of a pass that began at `started_at` and ran
    /// while the conversation was `idle` or not.
    pub fn push_compaction_marker(
        &mut self,
        model: &ModelSelection,
        boundary_id: BlockId,
        compacted_tokens: u64,
        idle: bool,
        started_at: Timestamp,
    ) {
        let (id, created_at) = self.stamp();
        self.append(Block::CompactionMarker(CompactionMarkerBlock {
            id,
            created_at,
            request: None,
            model: model.clone(),
            started_at: Some(started_at),
            boundary_id,
            compacted_tokens,
            idle,
        }));
    }

    /// Takes the blocks from `position` on off the path, as a Stop gives back
    /// a message whose turn wrote nothing of its answer. The place keeps its
    /// versions for the block that comes there next, as after Regenerate
    /// cut an answer; the save makes the cut a branch that stands in for
    /// what it took, not a version.
    pub fn withdraw_from(&mut self, position: usize) {
        if position < self.start || position >= self.len() {
            return;
        }
        self.blocks.truncate(position - self.start);
        self.versions.split_off(&(position + 1));
        self.journal.truncate(position);
        self.asking = self.held_turn();
        self.asked_end = None;
    }

    /// The stopped marker a Stop leaves.
    pub fn push_abort(&mut self, model: &ModelSelection) {
        let (id, created_at) = self.stamp();
        self.append(Block::Abort(AbortBlock {
            id,
            created_at,
            request: None,
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
            request: None,
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
            request: None,
            model: model.clone(),
            text,
            signature: None,
            ended_at: Some(created_at),
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
            let now = self.clock.now();
            open.text.push_str(text);
            open.ended_at = Some(now);
            self.journal.append_text(self.start + at, text, now);
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

    /// Where the running request's output starts: after the last input or
    /// response.
    fn request_start(&self) -> usize {
        self.blocks
            .iter()
            .rposition(|block| matches!(block, Block::User(_) | Block::Response(_)))
            .map_or(0, |at| at + 1)
    }

    /// Ends the running request's vendor steps that still run, as failed: the
    /// request ended without its response, so none of them will end.
    pub fn settle_opaque(&mut self) {
        let now = self.clock.now();
        for at in self.request_start()..self.blocks.len() {
            if let Block::Opaque(step) = &mut self.blocks[at]
                && step.status == OpaqueStatus::Running
            {
                step.status = OpaqueStatus::Failed;
                step.ended_at = Some(now);
                self.record_replace(self.start + at);
            }
        }
    }

    /// Starts a vendor's step, or updates the one of this request that has
    /// its call id: its status, view and end, and its entries after those it
    /// holds.
    pub fn apply_opaque(&mut self, model: &ModelSelection, step: OpaqueStep) {
        let now = self.clock.now();
        let ended_at = (step.status != OpaqueStatus::Running).then_some(now);
        let request_start = self.request_start();
        let started = step.call_id.as_ref().and_then(|call_id| {
            self.blocks[request_start..]
                .iter()
                .rposition(|block| matches!(block, Block::Opaque(open) if open.call_id.as_ref() == Some(call_id)))
                .map(|at| request_start + at)
        });
        if let Some(at) = started {
            if let Block::Opaque(open) = &mut self.blocks[at] {
                open.status = step.status;
                open.view = step.view;
                open.ended_at = ended_at;
                open.entries.extend(step.entries);
            }
            self.record_replace(self.start + at);
            return;
        }
        let (id, created_at) = self.stamp();
        self.append(Block::Opaque(OpaqueBlock {
            id,
            created_at,
            request: None,
            model: model.clone(),
            call_id: step.call_id,
            status: step.status,
            view: step.view,
            ended_at,
            run: None,
            entries: step.entries,
        }));
    }

    /// Adds answer text to the open text block, or opens one with it: a text
    /// block is open until it is marked complete.
    pub fn append_text(&mut self, model: &ModelSelection, text: &str) {
        if let Some(at) = self.blocks.len().checked_sub(1)
            && let Block::Text(open) = &mut self.blocks[at]
            && !open.forkable
        {
            let now = self.clock.now();
            open.text.push_str(text);
            open.ended_at = Some(now);
            self.journal.append_text(self.start + at, text, now);
            return;
        }
        let (id, created_at) = self.stamp();
        self.append(Block::Text(TextBlock {
            id,
            created_at,
            request: None,
            code_files: None,
            pictures: None,
            model: model.clone(),
            text: text.to_owned(),
            ended_at: Some(created_at),
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
        // The pictures of the Host the text shows are named in the same step
        // that completes it, so a page never sees a complete text whose
        // pictures are not yet named as being copied.
        if text.pictures.is_none() {
            let named = named_pictures(&text.text);
            if !named.is_empty() {
                text.pictures = Some(
                    named
                        .into_iter()
                        .map(|(target, _)| ReplyPicture { target, copy: PictureCopy::Copying })
                        .collect(),
                );
            }
        }
        self.record_replace(self.start + at);
        true
    }

    /// A requested call, saved as executing until the session completes it.
    ///
    /// Its input is kept as text: a vendor that could not parse the input hands
    /// its text over as a JSON string, and a call without input has an empty
    /// object. `writing_started_at` is when the model began writing it.
    pub fn push_tool_call(&mut self, model: &ModelSelection, call: ToolCall, writing_started_at: Option<Timestamp>) {
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
            request: None,
            model: model.clone(),
            tool_use_id: call.tool_use_id,
            tool_name: call.tool_name,
            input,
            status: ToolCallStatus::Executing,
            output: Vec::new(),
            view: None,
            writing_started_at,
            ended_at: None,
            entries: Vec::new(),
        }));
    }

    /// Records that the agent's account reached its limit and the request
    /// went on with another (`crates/provider-registry/src/rotation.rs`).
    pub fn push_account_switch(&mut self, model: &ModelSelection, switch: AccountSwitch) {
        let (id, created_at) = self.stamp();
        self.append(Block::AccountSwitch(AccountSwitchBlock {
            id,
            created_at,
            request: None,
            model: model.clone(),
            from: switch.from,
            to: switch.to,
            window: switch.window,
            resets_at: switch.resets_at,
        }));
    }

    /// Ends a request with the usage its provider reported, at the times
    /// it left and its first output came.
    pub fn push_response(&mut self, model: &ModelSelection, usage: Option<TokenUsage>, times: RequestTimes) {
        let RequestTimes {
            sent_at,
            first_sent_at,
            first_output_at,
        } = times;
        let (id, created_at) = self.stamp();
        self.append(Block::Response(ResponseBlock {
            id,
            created_at,
            request: None,
            model: model.clone(),
            sent_at: Some(sent_at),
            first_sent_at,
            first_output_at,
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

    /// Keeps `files` with the complete text `id` as which of its code spans
    /// name files, unless it keeps an answer already; answers whether it
    /// did. A block that is gone, as after an edit cut it, takes none.
    pub fn keep_code_files(&mut self, id: &BlockId, files: Vec<CodeFile>) -> bool {
        let Some(index) = self.position(id) else {
            return false;
        };
        let Block::Text(text) = &mut self.blocks[index - self.start] else {
            return false;
        };
        if !text.forkable || text.code_files.is_some() {
            return false;
        }
        text.code_files = Some(files);
        self.record_replace(index);
        true
    }

    /// Settles the picture the text `id` names by `target`, whose copy is
    /// being made, as `copy`; answers whether it did. A block that is gone,
    /// or a picture already settled, takes none.
    pub fn settle_picture(&mut self, id: &BlockId, target: &str, copy: PictureCopy) -> bool {
        let Some(index) = self.position(id) else {
            return false;
        };
        let Block::Text(text) = &mut self.blocks[index - self.start] else {
            return false;
        };
        let Some(picture) = text
            .pictures
            .iter_mut()
            .flatten()
            .find(|picture| picture.target == target && picture.copy == PictureCopy::Copying)
        else {
            return false;
        };
        picture.copy = copy;
        self.record_replace(index);
        true
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

    /// Appends `block`, which is no agent's message, with the run of steps
    /// it joins, which the blocks before it decide once.
    fn append(&mut self, block: Block) {
        // Only an agent's message reads where its sender stands.
        self.append_received(block, true);
    }

    /// [`append`](Self::append) for any block: an agent's message from a
    /// sender below this node, `sender_below`, is a step of the run it
    /// arrives in, and one from above or beside ends it.
    fn append_received(&mut self, mut block: Block, sender_below: bool) {
        block.set_run(run_of(&self.blocks, &block, sender_below));
        block.set_request(request_of(&self.blocks, &block));
        self.note_input(&block);
        let index = self.len();
        self.journal.add(index, &block, self.versions.get(&index).copied());
        self.blocks.push(block);
        self.note_end();
    }

    /// Decides, as an input of a turn arrives, whether the user asked for its
    /// turn ([`opens_asked`]); a steer of the user joins it to the user's.
    fn note_input(&mut self, block: &Block) {
        let Some(turn) = turn_of(block) else {
            return;
        };
        let same = self.asking.as_ref().is_some_and(|(current, _)| current == turn);
        let asked = match block {
            Block::Steer(_) => true,
            _ if same => return,
            _ => opens_asked(block, || matches!(self.user_input(), Some(None))),
        };
        self.asking = Some((turn.clone(), asked));
    }

    /// The last turn the held blocks have an input of, and whether the user
    /// asked for it, as [`note_input`](Self::note_input) would have decided
    /// it from its inputs in order; none when they hold no input.
    fn held_turn(&self) -> Option<(TurnId, bool)> {
        let last = self.blocks.iter().rposition(|block| turn_of(block).is_some())?;
        let turn = turn_of(&self.blocks[last])?.clone();
        let mut opener = last;
        let mut steered = false;
        for (index, block) in self.blocks[..=last].iter().enumerate().rev() {
            match turn_of(block) {
                Some(input) if *input == turn => {
                    opener = index;
                    steered |= matches!(block, Block::Steer(_));
                }
                Some(_) => break,
                None => {}
            }
        }
        let before = &self.blocks[..opener];
        let asked = steered
            || opens_asked(&self.blocks[opener], || {
                let input = before.iter().rev().find_map(crate::user_input).or_else(|| self.earlier.user_input().cloned());
                matches!(input, Some(None))
            });
        Some((turn, asked))
    }

    /// Keeps the end of a turn the user asked for, as it just arrived.
    fn note_end(&mut self) {
        let Some(end) = self.blocks.last() else {
            return;
        };
        let failed = match end {
            Block::Response(_) => false,
            Block::Error(_) if ends_turn(end) => true,
            _ => return,
        };
        // A log restored within a turn decides by what it holds.
        if self.asking.is_none() {
            self.asking = self.held_turn();
        }
        let asked = self.asking.as_ref().is_some_and(|(_, asked)| *asked);
        if asked {
            let answer = ended_turn_answer(&self.blocks).and_then(|(_, text)| text).map(|text| text.text.clone());
            self.asked_end = Some(AskedEnd { id: end.id().clone(), failed, answer });
        }
    }

    /// Takes the latest end of a turn the user asked for since the last
    /// take; none when no such turn ended.
    pub fn take_asked_end(&mut self) -> Option<AskedEnd> {
        self.asked_end.take()
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

/// Whether the user asked for the turn `block`, its first input, opens: the
/// user's own message or steer did, a schedule's message or another
/// conversation's did not, and any other input, a command's report, a
/// subagent's message, a resume, opens a turn of the work it continues,
/// which `continues_users` tells: whether the latest message of the user or
/// a schedule before it is the user's.
fn opens_asked(block: &Block, continues_users: impl FnOnce() -> bool) -> bool {
    match block {
        Block::Steer(_) => true,
        Block::User(user) => user.schedule.is_none(),
        Block::AgentMessage(message) if from_another_conversation(message) => false,
        _ => continues_users(),
    }
}
