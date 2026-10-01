//! Where history is cut: the recovery of an unfinished turn
//! (`failures-and-recovery.md` § Recovery is one mechanism), the prefix an
//! edit keeps (`message-editing.md`), the prefix a Fork keeps
//! (`conversation-fork.md`) and the window compaction summarizes
//! (`compaction.md` § One pass). Each is a pure reading of the blocks.

use demi_shared_types::{Block, BlockId, ToolCallStatus, TurnId, is_blank};

use super::{latest_answer, opens_input_turn, replay_start};

/// Where re-inference restarts after a turn failed to finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResumePoint {
    /// The blocks from this index on are the unfinished attempt's leftovers.
    pub cut: usize,
    /// The whole turn was leftovers: `resume` reruns it as `retry` does.
    pub full_rerun: bool,
}

/// Scans back from the end over what nobody can have acted on: thinking,
/// redacted thinking, `error` blocks and blank text. The first other block
/// stops the scan; reaching the block that opened the turn makes it a full
/// rerun.
pub fn resume_point(blocks: &[Block]) -> ResumePoint {
    for (index, block) in blocks.iter().enumerate().rev() {
        if opens_input_turn(block) {
            return ResumePoint {
                cut: index + 1,
                full_rerun: true,
            };
        }
        if !is_leftover(block) {
            return ResumePoint {
                cut: index + 1,
                full_rerun: false,
            };
        }
    }
    ResumePoint {
        cut: blocks.len(),
        full_rerun: false,
    }
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

/// The blocks through the completed text block `target`, which a Fork
/// keeps: none of them may be a call still executing.
pub fn through_assistant<'a>(
    blocks: &'a [Block],
    target: &BlockId,
) -> Result<&'a [Block], CutError> {
    let index = blocks
        .iter()
        .position(|block| block.id() == target)
        .filter(|index| matches!(&blocks[*index], Block::Text(text) if text.forkable))
        .ok_or(CutError::NotCompletedText)?;
    let prefix = &blocks[..=index];
    let executing = prefix.iter().any(
        |block| matches!(block, Block::ToolCall(call) if call.status == ToolCallStatus::Executing),
    );
    if executing {
        return Err(CutError::UnfinishedToolCalls);
    }
    Ok(prefix)
}

/// The blocks compaction summarizes (`compaction.md` § One pass): what the
/// session's latest answered request carried, from the last
/// `compaction_boundary`, or the first block, to the cut, where that
/// request's answer begins. With no request answered since the last
/// compaction, the window ends where the input no request answered begins,
/// which the pass keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionWindow {
    pub start: usize,
    pub cut: usize,
}

/// The window of the next pass over `blocks`.
pub fn compaction_window(blocks: &[Block]) -> CompactionWindow {
    let start = replay_start(blocks);
    let cut = latest_answer(blocks).unwrap_or_else(|| unanswered_input(blocks).max(start));
    CompactionWindow { start, cut }
}

/// Where the input no request has answered begins: at the last `user` block
/// after the latest `response` block, or right after that response when no
/// `user` block follows it; without a response, at the last `user` block,
/// or at the first block.
fn unanswered_input(blocks: &[Block]) -> usize {
    let answered = blocks
        .iter()
        .rposition(|block| matches!(block, Block::Response(_)))
        .map_or(0, |response| response + 1);
    blocks[answered..]
        .iter()
        .rposition(|block| matches!(block, Block::User(_)))
        .map_or(answered, |user| answered + user)
}

/// The text of the last `text` block from `since` on; empty when there is
/// none.
pub fn last_assistant_text(blocks: &[Block], since: usize) -> &str {
    blocks
        .get(since..)
        .unwrap_or_default()
        .iter()
        .rev()
        .find_map(|block| match block {
            Block::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .unwrap_or_default()
}
