//! Where history is cut: the recovery of an unfinished turn
//! (`failures-and-recovery.md` § Recovery is one mechanism), the prefix an
//! edit keeps (`message-editing.md`), the prefix a Fork keeps
//! (`conversation-fork.md`). Each is a pure reading of the blocks; the
//! window compaction summarizes is [`crate::compaction`]'s.

use demi_shared_types::{
    Block, BlockId, ErrorBlock, TextBlock, ToolCallStatus, TurnId, WakeupPlacement, is_blank,
};

use super::{is_compaction, opens_input_turn};

/// Where re-inference restarts after a turn failed to finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumePoint {
    /// The blocks from this index on are the unfinished attempt's leftovers,
    /// but for the compaction blocks among them, which stay ([`unwind`]).
    pub cut: usize,
    /// The turn whose opening block the scan reached: the whole turn was
    /// leftovers, and `resume` reruns it from that block as `retry` does.
    pub rerun: Option<TurnId>,
}

/// Scans back from the end over what nobody can have acted on: thinking,
/// redacted thinking, `error` blocks and blank text, passing over a
/// compaction's boundary and marker. The first other block stops the scan;
/// reaching the block that opened the turn makes it a rerun.
pub fn resume_point(blocks: &[Block]) -> ResumePoint {
    for (index, block) in blocks.iter().enumerate().rev() {
        if opens_input_turn(block) {
            return ResumePoint {
                cut: index + 1,
                rerun: turn_of(block).cloned(),
            };
        }
        if !is_leftover(block) && !is_compaction(block) {
            return ResumePoint {
                cut: index + 1,
                rerun: None,
            };
        }
    }
    ResumePoint {
        cut: blocks.len(),
        rerun: None,
    }
}

/// What `resume` keeps of `blocks` unwound to `cut`: the blocks before it,
/// and the compaction blocks after it, which are history the user made, not
/// leftovers.
pub fn unwind(blocks: &[Block], cut: usize) -> Vec<Block> {
    let compactions = blocks[cut..].iter().filter(|block| is_compaction(block));
    blocks[..cut].iter().chain(compactions).cloned().collect()
}

/// Whether a block is a leftover of an attempt that nobody can have acted on.
fn is_leftover(block: &Block) -> bool {
    match block {
        Block::Thinking(_) | Block::RedactedThinking(_) | Block::Error(_) => true,
        Block::Text(text) => is_blank(&text.text),
        _ => false,
    }
}

/// What `retry` keeps of a history: everything up to the block that opened
/// the last input turn, that block, and the turn's steers and every agent
/// message after it.
#[derive(Debug, Clone, PartialEq)]
pub struct Rewind {
    pub retained: Vec<Block>,
    /// The index of the input block in `retained`.
    pub input: usize,
    /// The turn the input block opened, which the rerun continues.
    pub turn: TurnId,
}

/// The rewind of the last input turn; none when the history has no input
/// turn. A turn is opened by a block that opens an input turn, or by the
/// first `agent_message` of a continuation.
pub fn rewind(blocks: &[Block]) -> Option<Rewind> {
    let mut seen = Vec::new();
    let mut start = None;
    for (index, block) in blocks.iter().enumerate() {
        let Some(turn) = turn_of(block) else {
            continue;
        };
        let first_of_turn = !seen.contains(&turn);
        if opens_input_turn(block) || (matches!(block, Block::AgentMessage(_)) && first_of_turn) {
            start = Some(index);
        }
        if first_of_turn {
            seen.push(turn);
        }
    }
    let start = start?;
    let turn = turn_of(&blocks[start])
        .expect("an input block belongs to a turn")
        .clone();
    let kept_after = blocks[start + 1..].iter().filter(|block| match block {
        Block::AgentMessage(_) => true,
        Block::Steer(steer) => steer.turn_id == turn,
        _ => false,
    });
    let retained = blocks[..=start].iter().chain(kept_after).cloned().collect();
    Some(Rewind {
        retained,
        input: start,
        turn,
    })
}

/// The turn a block of a turn's input belongs to.
fn turn_of(block: &Block) -> Option<&TurnId> {
    match block {
        Block::User(block) => Some(&block.turn_id),
        Block::Context(block) => Some(&block.turn_id),
        Block::Wakeup(block) => Some(&block.turn_id),
        Block::Steer(block) => Some(&block.turn_id),
        Block::AgentMessage(block) => Some(&block.turn_id),
        Block::Resume(block) => Some(&block.turn_id),
        _ => None,
    }
}

/// Why a history cannot be cut where a request asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CutError {
    #[error("The edit target must be a user message")]
    NotUserMessage,
    #[error("The Fork target must be a completed assistant message")]
    NotCompletedText,
    #[error("The Fork boundary contains unfinished tool calls")]
    UnfinishedToolCalls,
    #[error("No completed assistant message to fork from yet")]
    NothingCompleted,
}

/// Where a Fork's history ends (`conversation-fork.md` § Eligibility).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkEnd<'a> {
    /// The completed text the user picked, which a conversation Fork names.
    At(&'a BlockId),
    /// The newest completed text with no call before it still executing,
    /// which `demi agent fork` takes (`subagents.md` § Fork).
    Newest,
}

/// The blocks before the `user` block `target`, which an edit keeps.
pub fn before_user<'a>(blocks: &'a [Block], target: &BlockId) -> Result<&'a [Block], CutError> {
    let index = blocks
        .iter()
        .position(|block| block.id() == target)
        .filter(|index| blocks[*index].is_editable())
        .ok_or(CutError::NotUserMessage)?;
    Ok(&blocks[..index])
}

/// The blocks a Fork keeps, through the completed text block `end` names:
/// none of them may be a call still executing.
pub fn fork_prefix<'a>(blocks: &'a [Block], end: ForkEnd<'_>) -> Result<&'a [Block], CutError> {
    let is_completed = |block: &Block| matches!(block, Block::Text(text) if text.forkable);
    let index = match end {
        ForkEnd::At(target) => blocks
            .iter()
            .position(|block| block.id() == target)
            .filter(|index| is_completed(&blocks[*index]))
            .ok_or(CutError::NotCompletedText)?,
        ForkEnd::Newest => {
            let executing = blocks.iter().position(is_executing).unwrap_or(blocks.len());
            blocks[..executing]
                .iter()
                .rposition(is_completed)
                .ok_or(CutError::NothingCompleted)?
        }
    };
    let prefix = &blocks[..=index];
    if prefix.iter().any(is_executing) {
        return Err(CutError::UnfinishedToolCalls);
    }
    Ok(prefix)
}

fn is_executing(block: &Block) -> bool {
    matches!(block, Block::ToolCall(call) if call.status == ToolCallStatus::Executing)
}

/// The last `text` block from `since` on, the agent's last answer.
pub fn last_answer(blocks: &[Block], since: usize) -> Option<&TextBlock> {
    blocks
        .get(since..)
        .unwrap_or_default()
        .iter()
        .rev()
        .find_map(|block| match block {
            Block::Text(text) => Some(text),
            _ => None,
        })
}

/// The record that ended the last turn (`failures-and-recovery.md` § The
/// unfinished turn): the last block but a compaction's boundary and marker,
/// an error that ended no turn, such as a Compact whose summary request
/// failed, and the agent messages and command reports a failed turn took
/// after it ended, which leave the turn before them as it was. An `error`
/// block there is the failure that left the turn unfinished.
pub fn turn_record(blocks: &[Block]) -> Option<&Block> {
    blocks.iter().rev().find(|block| {
        !is_compaction(block)
            && !matches!(block, Block::Error(error) if error.outside_turn)
            && !is_late_input(block)
    })
}

/// Whether a block is input a turn takes without opening one: an agent
/// message, or command reports placed as a steer.
fn is_late_input(block: &Block) -> bool {
    match block {
        Block::AgentMessage(_) => true,
        Block::Wakeup(wakeup) => wakeup.placement == WakeupPlacement::Steer,
        _ => false,
    }
}

/// The turn the last block of a turn's input belongs to: the last turn.
pub fn last_turn(blocks: &[Block]) -> Option<&TurnId> {
    blocks.iter().rev().find_map(turn_of)
}

/// The last `error` block, the record of the agent's last failure.
pub fn last_failure(blocks: &[Block]) -> Option<&ErrorBlock> {
    blocks.iter().rev().find_map(|block| match block {
        Block::Error(error) => Some(error),
        _ => None,
    })
}

/// The text of the last `text` block from `since` on; empty when there is
/// none.
pub fn last_assistant_text(blocks: &[Block], since: usize) -> &str {
    last_answer(blocks, since).map_or("", |text| text.text.as_str())
}
