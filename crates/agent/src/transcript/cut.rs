//! Where history is cut: the recovery of an unfinished turn
//! (`failures-and-recovery.md` § Recovery is one mechanism), the prefix an
//! edit keeps (`message-editing.md`), the prefix a Fork keeps
//! (`conversation-fork.md`) and the window compaction summarizes
//! (`compaction.md` § One pass). Each is a pure reading of the blocks.

use demi_core::{Block, BlockId, ToolCallStatus, TurnId, is_blank};

use super::{estimate::block_tokens, opens_input_turn};

/// Where re-inference restarts after a turn failed to finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResumePoint {
    /// The blocks from this index on are the unfinished attempt's leftovers.
    pub(crate) cut: usize,
    /// The whole turn was leftovers: `resume` reruns it as `retry` does.
    pub(crate) full_rerun: bool,
}

/// Scans back from the end over what nobody can have acted on: thinking,
/// redacted thinking, `error` blocks and blank text. The first other block
/// stops the scan; reaching the block that opened the turn makes it a full
/// rerun.
pub(crate) fn resume_point(blocks: &[Block]) -> ResumePoint {
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
pub(crate) struct Rewind {
    pub(crate) retained: Vec<Block>,
    /// The index of the input block in `retained`.
    pub(crate) input: usize,
    /// The turn the input block opened, which the rerun continues.
    pub(crate) turn: TurnId,
}

/// The rewind of the last input turn; none when the history has no input
/// turn. A turn is opened by a block that opens an input turn, or by the
/// first `agent_message` of a continuation.
pub(crate) fn rewind(blocks: &[Block]) -> Option<Rewind> {
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
pub(crate) fn before_user<'a>(
    blocks: &'a [Block],
    target: &BlockId,
) -> Result<&'a [Block], CutError> {
    let index = blocks
        .iter()
        .position(|block| block.id() == target)
        .filter(|index| blocks[*index].is_editable())
        .ok_or(CutError::NotUserMessage)?;
    Ok(&blocks[..index])
}

/// The blocks through the completed text block `target`, which a Fork
/// keeps: none of them may be a call still executing.
pub(crate) fn through_assistant<'a>(
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

/// The blocks compaction summarizes: from the last `compaction_boundary`,
/// or the first block, to the cut, before which counting back from the end
/// the kept history reaches `keep_recent_tokens`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompactionWindow {
    pub(crate) start: usize,
    pub(crate) cut: usize,
}

/// The window of the next pass; none when the history is too short to keep
/// `keep_recent_tokens`. A cut that lands on a `response` falls after it, so
/// a request's usage stays with the history it measured.
pub(crate) fn compaction_window(
    blocks: &[Block],
    keep_recent_tokens: u64,
) -> Option<CompactionWindow> {
    let start = blocks
        .iter()
        .rposition(|block| matches!(block, Block::CompactionBoundary(_)))
        .unwrap_or(0);
    let mut recent = 0;
    for index in (start + 1..blocks.len()).rev() {
        recent += block_tokens(&blocks[index]);
        if recent >= keep_recent_tokens {
            let cut = match blocks[index] {
                Block::Response(_) => index + 1,
                _ => index,
            };
            return Some(CompactionWindow { start, cut });
        }
    }
    None
}

/// The text of the last `text` block from `since` on; empty when there is
/// none.
pub(crate) fn last_assistant_text(blocks: &[Block], since: usize) -> &str {
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

#[cfg(test)]
mod tests {
    use demi_core::{
        AbortBlock, ErrorBlock, ResponseBlock, TextBlock, ThinkingBlock, Timestamp, TokenUsage,
        UserBlock,
    };

    use super::*;
    use crate::testing::test_model;

    fn id(value: &str) -> BlockId {
        BlockId::try_from(value).unwrap()
    }

    fn user(value: &str) -> Block {
        Block::User(UserBlock {
            id: id(value),
            turn_id: TurnId::try_from(value).unwrap(),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            content: Vec::new(),
            preamble: None,
        })
    }

    fn text(value: &str) -> Block {
        Block::Text(TextBlock {
            id: id("text"),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            text: value.into(),
            forkable: false,
        })
    }

    fn thinking() -> Block {
        Block::Thinking(ThinkingBlock {
            id: id("thinking"),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            text: "hmm".into(),
            signature: None,
        })
    }

    fn error() -> Block {
        Block::Error(ErrorBlock {
            id: id("error"),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            message: "failed".into(),
            code: None,
            diagnostics: None,
        })
    }

    fn response() -> Block {
        Block::Response(ResponseBlock {
            id: id("response"),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            usage: TokenUsage::default(),
        })
    }

    fn abort() -> Block {
        Block::Abort(AbortBlock {
            id: id("abort"),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            is_resumed: false,
        })
    }

    #[test]
    fn the_resume_point_drops_only_what_nobody_can_have_acted_on() {
        let point = |blocks: &[Block]| {
            let point = resume_point(blocks);
            (point.cut, point.full_rerun)
        };
        // The failure alone: the turn reruns.
        assert_eq!(point(&[user("u1"), thinking(), error()]), (1, true));
        // Blank text is a leftover; emitted text, a response and a stop are
        // not.
        assert_eq!(point(&[user("u1"), text(" \n"), error()]), (1, true));
        assert_eq!(point(&[user("u1"), text("posted"), error()]), (2, false));
        assert_eq!(point(&[user("u1"), response(), thinking()]), (2, false));
        assert_eq!(point(&[user("u1"), abort()]), (2, false));
        // Only the latest turn is in play.
        assert_eq!(
            point(&[user("u1"), text("answer"), response(), user("u2"), error()]),
            (4, true)
        );
        assert_eq!(point(&[]), (0, false));
    }
}
