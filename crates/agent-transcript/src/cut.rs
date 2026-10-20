//! Where history is cut: the recovery of an unfinished turn, the prefix an
//! edit keeps, the prefix a Fork keeps. Each is a pure reading of the
//! blocks; the window compaction summarizes is [`crate::compaction`]'s.

use demi_shared_types::{
    Block, BlockId, ErrorBlock, TextBlock, ToolCallStatus, TurnId, is_blank,
};

use demi_agent_store::Kind;

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
///
/// Recovery is this one mechanism: a transient provider failure and a human
/// asking to continue a dead turn ask the same question, how the turn gets
/// finished, and the answer depends only on what has already left the
/// process, never on which of the two asked or how the turn died. A caller
/// must not branch on the failure kind: a terminal error can arrive after ten
/// minutes of tool calls as easily as before the first token, so mapping an
/// error to one action and a stop to another picks the destructive path
/// exactly when it is most damaging. The scan stops at the first block
/// someone may already have acted on, since blocks stream outward as they
/// are produced and become effects that cannot be recalled: rendering,
/// posting, running the tool. A tool call stops it whatever its status, as
/// one still executing outlived the process and whether its effect landed is
/// unknown. A rerun spares the model a continuation attached to a stub of its
/// own aborted output. `retry` ([`rewind`]) is not recovery: it discards the
/// whole turn for a caller who wants another answer and knows its effects can
/// be repeated.
pub fn resume_point(blocks: &[Block]) -> ResumePoint {
    resume_point_within(blocks).unwrap_or(ResumePoint {
        cut: blocks.len(),
        rerun: None,
    })
}

/// The resume point as [`resume_point`] finds it, when a block of `blocks`
/// stops the scan; none when the scan passes all of them, as it passes a
/// window that holds only a compaction and leftovers.
pub fn resume_point_within(blocks: &[Block]) -> Option<ResumePoint> {
    for (index, block) in blocks.iter().enumerate().rev() {
        if opens_input_turn(block) {
            return Some(ResumePoint {
                cut: index + 1,
                rerun: turn_of(block).cloned(),
            });
        }
        if !is_leftover(block) && !is_compaction(block) {
            return Some(ResumePoint {
                cut: index + 1,
                rerun: None,
            });
        }
    }
    None
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
pub fn turn_of(block: &Block) -> Option<&TurnId> {
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
    #[error("The conversation holds no such block")]
    NotFound,
    #[error("The edit target must be a user message")]
    NotUserMessage,
    #[error("The Fork target must be a completed assistant message")]
    NotCompletedText,
    #[error("The Fork boundary contains unfinished tool calls")]
    UnfinishedToolCalls,
    #[error("No completed assistant message to fork from yet")]
    NothingCompleted,
}

/// Where a Fork's history ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkEnd<'a> {
    /// The completed text the user picked, which a conversation Fork names.
    At(&'a BlockId),
    /// The newest completed text with no call before it still executing,
    /// which `demi agent fork` takes (`crates/agent-server/src/server/tree/supervisor.rs`).
    Newest,
    /// The completed text a side question asks about, with the response
    /// that ended it; see [`fork_prefix`].
    Side(&'a BlockId),
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
    let at = |target: &BlockId| {
        blocks
            .iter()
            .position(|block| block.id() == target)
            .filter(|index| is_completed(&blocks[*index]))
            .ok_or(CutError::NotCompletedText)
    };
    let index = match end {
        ForkEnd::At(target) => at(target)?,
        ForkEnd::Newest => newest(blocks, 0).ok_or(CutError::NothingCompleted)?,
        // A side question's history is what the conversation's next request
        // would have sent after the reply, its response included, so it
        // reads the conversation's prompt cache. Once a compaction
        // summarized the reply, the reply is only in the summary, and the
        // history is the conversation's context as it stands: the summary
        // and what follows it, through the newest completed text, or
        // through the compaction when no text followed it yet.
        ForkEnd::Side(target) => side_end(blocks, at(target)?),
    };
    let prefix = &blocks[..=index];
    if prefix.iter().any(is_executing) {
        return Err(CutError::UnfinishedToolCalls);
    }
    Ok(prefix)
}

/// Where a side question's history ends among `blocks` when it asks
/// about the text at `asked` ([`ForkEnd::Side`]): there, with its
/// response, or, once a compaction after it summarized it, at the newest
/// completed text after the last compaction, or at that compaction.
/// `blocks` may be a window that starts after the text, with `asked` 0:
/// the window starts with the compaction that summarized it.
pub fn side_end(blocks: &[Block], asked: usize) -> usize {
    let end = match blocks[asked..].iter().rposition(is_compaction) {
        None => asked,
        Some(after) => {
            let compaction = asked + after;
            newest(blocks, compaction).unwrap_or(compaction)
        }
    };
    match blocks.get(end + 1) {
        Some(Block::Response(_)) if is_completed(&blocks[end]) => end + 1,
        _ => end,
    }
}

/// The newest completed text of `blocks` from `from` on with no call
/// before it still executing.
fn newest(blocks: &[Block], from: usize) -> Option<usize> {
    let executing = blocks.iter().position(is_executing).unwrap_or(blocks.len());
    blocks
        .get(from..executing)
        .unwrap_or_default()
        .iter()
        .rposition(is_completed)
        .map(|index| from + index)
}

fn is_completed(block: &Block) -> bool {
    matches!(block, Block::Text(text) if text.forkable)
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

/// The record that ended the last turn: the last block but a compaction's
/// boundary and marker, an error that ended no turn, such as a Compact
/// whose summary request failed, and the agent messages and command reports
/// a failed turn took after it ended, which leave the turn before them as
/// it was. An `error` block there is the failure that left the turn
/// unfinished.
pub fn turn_record(blocks: &[Block]) -> Option<&Block> {
    blocks.iter().rev().find(|block| Kind::Record.holds(block))
}

/// The block that ended the latest ended turn, as a conversation's summary
/// names it (`tree::summary` in the backend's storage): the last `response`,
/// `error` or `abort` but an error that ended no turn and a stop a Resume
/// continued.
pub fn ended_turn(blocks: &[Block]) -> Option<&Block> {
    blocks.iter().rev().find(|block| match block {
        Block::Response(_) => true,
        Block::Error(error) => !error.outside_turn,
        Block::Abort(abort) => !abort.is_resumed,
        _ => false,
    })
}

/// The latest ended turn: the block that ended it, as [`ended_turn`] finds
/// it, and its last text that is not blank, the answer a conversation's
/// summary names. A text belongs to the turn when it comes after one of
/// that turn's input blocks and before the end.
pub fn ended_turn_answer(blocks: &[Block]) -> Option<(&Block, Option<&TextBlock>)> {
    let end = ended_turn(blocks)?;
    let at = blocks.iter().rposition(|block| std::ptr::eq(block, end))?;
    let mut turn: Option<&TurnId> = None;
    let mut answer = None;
    for block in blocks[..at].iter().rev() {
        if let Some(input) = turn_of(block) {
            match turn {
                Some(turn) if turn != input => break,
                _ => turn = Some(input),
            }
            continue;
        }
        if answer.is_none()
            && let Block::Text(text) = block
            && !is_blank(&text.text)
        {
            answer = Some(text);
        }
    }
    // A text with no input of the turn before it belongs to none.
    Some((end, answer.filter(|_| turn.is_some())))
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
