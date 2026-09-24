//! The transcript itself: its blocks in order, changed only through methods
//! that record each change in the journal, and its version.

use std::{rc::Rc, sync::Arc};

use demi_agent_protocol::TranscriptVersion;
use demi_core::{
    AbortBlock, AgentMessage, AgentMessageBlock, Block, BlockId, Clock, CompactionBoundaryBlock,
    CompactionMarkerBlock, ContextBlock, ErrorBlock, ModelSelection, ProviderErrorDiagnostics,
    RedactedThinkingBlock, ResponseBlock, ResumeBlock, SteerBlock, TextBlock, ThinkingBlock,
    Timestamp, TokenUsage, ToolCallBlock, ToolCallStatus, ToolResultContentBlock, ToolView, TurnId,
    UserBlock, UserContentBlock, WakeupBlock, WakeupPlacement,
};
use demi_provider::ToolCall;
use serde_json::Value;

use super::{INTERRUPTED_CODE, PatchBatch, journal::Journal};
use crate::IdSource;

/// A tool call the provider requested that has no result yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingCall {
    pub(crate) tool_use_id: String,
    pub(crate) tool_name: String,
    /// The call's input as the JSON text the provider supplied.
    pub(crate) input: String,
}

pub(crate) struct TranscriptLog {
    blocks: Vec<Block>,
    journal: Journal,
    revision: u64,
    /// New for every log, at creation and at every restore, so a version
    /// taken before a restart never matches after it.
    epoch: String,
    ids: Rc<dyn IdSource>,
    clock: Arc<dyn Clock>,
}

impl TranscriptLog {
    pub(crate) fn new(blocks: Vec<Block>, ids: Rc<dyn IdSource>, clock: Arc<dyn Clock>) -> Self {
        let epoch = ids.next_id();
        Self {
            blocks,
            journal: Journal::default(),
            revision: 0,
            epoch,
            ids,
            clock,
        }
    }

    pub(crate) fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub(crate) fn version(&self) -> TranscriptVersion {
        TranscriptVersion {
            epoch: self.epoch.clone(),
            revision: self.revision,
        }
    }

    /// The block with this id, looked for from the end, where recent changes
    /// are.
    pub(crate) fn find(&self, id: &BlockId) -> Option<&Block> {
        self.blocks.iter().rev().find(|block| block.id() == id)
    }

    /// The patches recorded since the last call, as one batch that advances
    /// the revision; none when nothing changed.
    pub(crate) fn take_patches(&mut self) -> Option<PatchBatch> {
        let (patches, touched) = self.journal.take()?;
        self.revision += 1;
        Some(PatchBatch {
            revision: self.revision,
            patches,
            touched,
        })
    }

    pub(crate) fn push_user(
        &mut self,
        turn_id: TurnId,
        model: &ModelSelection,
        content: Vec<UserContentBlock>,
        preamble: Option<String>,
    ) -> BlockId {
        let (id, created_at) = self.stamp();
        self.append(Block::User(UserBlock {
            id: id.clone(),
            turn_id,
            created_at,
            model: model.clone(),
            content,
            preamble,
        }));
        id
    }

    pub(crate) fn push_context(&mut self, turn_id: TurnId, model: &ModelSelection, text: String) {
        let (id, created_at) = self.stamp();
        self.append(Block::Context(ContextBlock {
            id,
            turn_id,
            created_at,
            model: model.clone(),
            text,
        }));
    }

    /// A human steer, under the steer's id.
    pub(crate) fn push_steer(
        &mut self,
        id: BlockId,
        turn_id: TurnId,
        model: &ModelSelection,
        content: Vec<UserContentBlock>,
    ) {
        let created_at = self.clock.now();
        self.append(Block::Steer(SteerBlock {
            id,
            turn_id,
            created_at,
            model: model.clone(),
            content,
        }));
    }

    /// A fired yield wakeup, under the wakeup's id.
    pub(crate) fn push_wakeup(
        &mut self,
        id: BlockId,
        turn_id: TurnId,
        model: &ModelSelection,
        placement: WakeupPlacement,
    ) {
        let created_at = self.clock.now();
        self.append(Block::Wakeup(WakeupBlock {
            id,
            turn_id,
            created_at,
            model: model.clone(),
            placement,
        }));
    }

    /// An agent message, under the message's id.
    pub(crate) fn push_agent_message(
        &mut self,
        turn_id: TurnId,
        model: &ModelSelection,
        message: AgentMessage,
    ) {
        let created_at = self.clock.now();
        self.append(Block::AgentMessage(AgentMessageBlock {
            id: message.id.clone(),
            turn_id,
            created_at,
            model: model.clone(),
            message,
        }));
    }

    /// The turn continues after a cut.
    pub(crate) fn push_resume(&mut self, turn_id: TurnId, model: &ModelSelection) {
        let (id, created_at) = self.stamp();
        self.append(Block::Resume(ResumeBlock {
            id,
            turn_id,
            created_at,
            model: model.clone(),
        }));
    }

    /// Marks the latest stopped marker as continued.
    pub(crate) fn mark_latest_abort_resumed(&mut self) {
        let Some(index) = self
            .blocks
            .iter()
            .rposition(|block| matches!(block, Block::Abort(_)))
        else {
            return;
        };
        if let Block::Abort(abort) = &mut self.blocks[index] {
            abort.is_resumed = true;
        }
        self.record_replace(index);
    }

    /// Replaces every block with `blocks`, as a history rewrite publishes
    /// its retained history: one `replace` patch.
    pub(crate) fn replace_all(&mut self, blocks: Vec<Block>) {
        self.blocks = blocks;
        self.journal.replace_all(&self.blocks);
    }

    /// Inserts compaction's summary at `index`, where the kept history
    /// begins, and returns its id.
    pub(crate) fn insert_compaction_boundary(
        &mut self,
        index: usize,
        model: &ModelSelection,
        summary: String,
        summary_tokens: u64,
    ) -> BlockId {
        let (id, created_at) = self.stamp();
        let block = Block::CompactionBoundary(CompactionBoundaryBlock {
            id: id.clone(),
            created_at,
            model: model.clone(),
            summary,
            summary_tokens,
        });
        self.journal.add(index, &block);
        self.blocks.insert(index, block);
        id
    }

    /// Appends the marker of the boundary `boundary_id` with the estimate of
    /// what it summarized.
    pub(crate) fn push_compaction_marker(
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
    pub(crate) fn push_abort(&mut self, model: &ModelSelection) {
        let (id, created_at) = self.stamp();
        self.append(Block::Abort(AbortBlock {
            id,
            created_at,
            model: model.clone(),
            is_resumed: false,
        }));
    }

    pub(crate) fn push_error(
        &mut self,
        model: &ModelSelection,
        message: String,
        code: Option<String>,
        diagnostics: Option<ProviderErrorDiagnostics>,
    ) {
        let (id, created_at) = self.stamp();
        self.append(Block::Error(ErrorBlock {
            id,
            created_at,
            model: model.clone(),
            message,
            code,
            diagnostics,
        }));
    }

    /// Whether a `user` block of the turn `turn` exists: the turn began.
    pub(crate) fn has_user_turn(&self, turn: &TurnId) -> bool {
        self.blocks
            .iter()
            .any(|block| matches!(block, Block::User(user) if &user.turn_id == turn))
    }

    /// Whether the last block already says why its turn is unfinished.
    pub(crate) fn ends_with_interruption(&self) -> bool {
        matches!(
            self.blocks.last(),
            Some(Block::Error(error)) if error.code.as_deref() == Some(INTERRUPTED_CODE)
        )
    }

    /// Opens a reasoning block with `text`, as a provider's thinking start
    /// does.
    pub(crate) fn open_thinking(&mut self, model: &ModelSelection, text: String) {
        let (id, created_at) = self.stamp();
        self.append(Block::Thinking(ThinkingBlock {
            id,
            created_at,
            model: model.clone(),
            text,
            signature: None,
        }));
    }

    /// Adds reasoning text to the open reasoning block, or opens one with
    /// it: a block is open until the vendor signed it.
    pub(crate) fn append_thinking(&mut self, model: &ModelSelection, text: &str) {
        if let Some(index) = self.blocks.len().checked_sub(1)
            && let Block::Thinking(open) = &mut self.blocks[index]
            && open.signature.is_none()
        {
            open.text.push_str(text);
            let id = open.id.clone();
            self.journal.append_text(index, &id, text);
            return;
        }
        self.open_thinking(model, text.to_owned());
    }

    /// Signs the latest reasoning block. A signature with no reasoning block
    /// before it has nothing to sign and is dropped.
    pub(crate) fn sign_thinking(&mut self, signature: String) {
        let Some(index) = self
            .blocks
            .iter()
            .rposition(|block| matches!(block, Block::Thinking(_)))
        else {
            return;
        };
        if let Block::Thinking(thinking) = &mut self.blocks[index] {
            thinking.signature = Some(signature);
        }
        self.record_replace(index);
    }

    pub(crate) fn push_redacted_thinking(&mut self, model: &ModelSelection, data: String) {
        let (id, created_at) = self.stamp();
        self.append(Block::RedactedThinking(RedactedThinkingBlock {
            id,
            created_at,
            model: model.clone(),
            data,
        }));
    }

    /// Adds answer text to the open text block, or opens one with it: a text
    /// block is open until it is marked complete.
    pub(crate) fn append_text(&mut self, model: &ModelSelection, text: &str) {
        if let Some(index) = self.blocks.len().checked_sub(1)
            && let Block::Text(open) = &mut self.blocks[index]
            && !open.forkable
        {
            open.text.push_str(text);
            let id = open.id.clone();
            self.journal.append_text(index, &id, text);
            return;
        }
        let (id, created_at) = self.stamp();
        self.append(Block::Text(TextBlock {
            id,
            created_at,
            model: model.clone(),
            text: text.to_owned(),
            forkable: false,
        }));
    }

    /// Whether the last block is answer text that is not yet complete.
    pub(crate) fn ends_with_open_text(&self) -> bool {
        matches!(self.blocks.last(), Some(Block::Text(text)) if !text.forkable)
    }

    /// Marks the last block's text complete, so a Fork may start after it,
    /// and returns its id. Text that follows a call still executing cannot be
    /// a Fork's end, so it stays as it is.
    pub(crate) fn complete_tail_text(&mut self) -> Option<BlockId> {
        let index = self.blocks.len().checked_sub(1)?;
        let (earlier, tail) = self.blocks.split_at_mut(index);
        let Block::Text(text) = &mut tail[0] else {
            return None;
        };
        let executing = earlier.iter().any(
            |block| matches!(block, Block::ToolCall(call) if call.status == ToolCallStatus::Executing),
        );
        if text.forkable || executing {
            return None;
        }
        text.forkable = true;
        let id = text.id.clone();
        self.record_replace(index);
        Some(id)
    }

    /// A requested call, saved as executing until the session completes it.
    /// Its input is kept as text: a vendor that could not parse the input
    /// hands its text over as a JSON string, and a call without input has an
    /// empty object.
    pub(crate) fn push_tool_call(&mut self, model: &ModelSelection, call: ToolCall) {
        let input = match call.input {
            Value::String(text) => text,
            Value::Null => "{}".to_owned(),
            other => other.to_string(),
        };
        let (id, created_at) = self.stamp();
        self.append(Block::ToolCall(ToolCallBlock {
            id,
            created_at,
            model: model.clone(),
            tool_use_id: call.tool_use_id,
            tool_name: call.tool_name,
            input,
            status: ToolCallStatus::Executing,
            output: Vec::new(),
            view: None,
        }));
    }

    pub(crate) fn push_response(&mut self, model: &ModelSelection, usage: TokenUsage) {
        let (id, created_at) = self.stamp();
        self.append(Block::Response(ResponseBlock {
            id,
            created_at,
            model: model.clone(),
            usage,
        }));
    }

    /// Every call still executing, in order.
    pub(crate) fn pending_tool_calls(&self) -> Vec<PendingCall> {
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
    pub(crate) fn complete_tool_call(
        &mut self,
        tool_use_id: &str,
        output: Vec<ToolResultContentBlock>,
        is_error: bool,
        view: Option<ToolView>,
    ) {
        let Some(index) = self.blocks.iter().rposition(|block| {
            matches!(block, Block::ToolCall(call)
                if call.status == ToolCallStatus::Executing && call.tool_use_id == tool_use_id)
        }) else {
            return;
        };
        if let Block::ToolCall(call) = &mut self.blocks[index] {
            call.status = if is_error {
                ToolCallStatus::Error
            } else {
                ToolCallStatus::Completed
            };
            call.output = output;
            call.view = view;
        }
        self.record_replace(index);
    }

    /// A new block's id and time.
    fn stamp(&self) -> (BlockId, Timestamp) {
        let id = BlockId::try_from(self.ids.next_id())
            .expect("an id source never gives an empty identity");
        (id, self.clock.now())
    }

    fn append(&mut self, block: Block) {
        self.journal.add(self.blocks.len(), &block);
        self.blocks.push(block);
    }

    fn record_replace(&mut self, index: usize) {
        self.journal.replace_block(index, &self.blocks[index]);
    }
}

#[cfg(test)]
mod tests {
    use std::{rc::Rc, sync::Arc};

    use demi_core::{Block, Timestamp, ToolCallStatus, ToolResultContentBlock};
    use demi_provider::{ToolCall, testing::FixedClock};
    use serde_json::json;

    use super::TranscriptLog;
    use crate::testing::{SequentialIds, test_model};

    #[test]
    fn a_reused_tool_use_id_completes_the_call_that_waits() {
        let mut log = TranscriptLog::new(
            Vec::new(),
            Rc::new(SequentialIds::new("b")),
            Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
        );
        let call = || ToolCall {
            tool_use_id: "call-1".into(),
            tool_name: "shell_exec".into(),
            input: json!({}),
        };
        let output = |text: &str| vec![ToolResultContentBlock::Text { text: text.into() }];
        log.push_tool_call(&test_model(), call());
        log.complete_tool_call("call-1", output("first"), false, None);
        log.push_tool_call(&test_model(), call());
        log.complete_tool_call("call-1", output("second"), false, None);

        let outputs: Vec<_> = log
            .blocks()
            .iter()
            .map(|block| match block {
                Block::ToolCall(call) => (call.status, call.output.clone()),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            outputs,
            [
                (ToolCallStatus::Completed, output("first")),
                (ToolCallStatus::Completed, output("second")),
            ]
        );
    }
}
