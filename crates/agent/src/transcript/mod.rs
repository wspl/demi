//! A session's transcript (`runtime.md` § Transcript): its blocks, the
//! patches that record every change to them, the model's replay of them, and
//! the token estimates compaction decides by.

mod cut;
pub mod estimate;
mod journal;
mod log;
mod replay;
pub mod retire;

pub use cut::CutError;
pub(crate) use cut::{
    before_user, compaction_window, last_assistant_text, resume_point, rewind, through_assistant,
};
pub(crate) use journal::{DirtyRows, PatchBatch};
pub(crate) use log::TranscriptLog;
#[cfg(test)]
pub(crate) use replay::agent_message_envelope;
pub(crate) use replay::{char_offset, replay, tool_input};

use demi_core::{Block, GoneCause, ModelMediaKind, WakeupPlacement};
use jiff::tz::TimeZone;

/// What the model receives for a `resume` block.
pub(crate) const RESUME_TEXT: &str = "Continue from where you left off.";

/// What the model receives for a tool's medium that is gone (`runtime.md`
/// § Media): `[<kind> not stored: <reason>]` when its bytes could not be
/// stored, and, once retired, `[<kind>:<media type>, removed on <day>: a
/// tool result's images and videos are kept for 30 days]` with the UTC day
/// of the retirement.
pub(crate) fn gone_text(kind: ModelMediaKind, media_type: &str, cause: &GoneCause) -> String {
    let kind = kind.name();
    match cause {
        GoneCause::NotStored { error } => format!("[{kind} not stored: {error}]"),
        GoneCause::Retired { at } => format!(
            "[{kind}:{media_type}, removed on {}: a tool result's images and videos are kept for {} days]",
            at.to_jiff().to_zoned(TimeZone::UTC).date(),
            retire::KEPT.as_hours() / 24
        ),
    }
}

/// What the model receives for a fired yield wakeup.
pub(crate) const WAKEUP_TEXT: &str = "Scheduled yield wakeup fired. Continue the previous work and inspect any running command with shell_status when needed.";

/// What the error record of a turn the session was shut down under says.
pub(crate) const INTERRUPTED_TURN_MESSAGE: &str =
    "The agent session was shut down while this turn was running.";

/// The code of that record.
pub(crate) const INTERRUPTED_CODE: &str = "interrupted";

/// Where replay starts in `blocks` (`runtime.md` § Replay): at the last
/// `compaction_boundary`, or at the first block when there is none.
pub(crate) fn replay_start(blocks: &[Block]) -> usize {
    blocks
        .iter()
        .rposition(|block| matches!(block, Block::CompactionBoundary(_)))
        .unwrap_or(0)
}

/// Where the answer to the session's latest answered request begins
/// (`runtime.md` § Replay): that request's `response` block comes last,
/// after the last `compaction_marker`, and its answer is the run of
/// thinking, text and tool-call blocks directly before that `response`
/// block. What the blocks before the answer replay is what the request
/// carried. None when no request was answered since the last compaction.
/// A boundary whose marker an edit cut counts as the last compaction.
pub(crate) fn latest_answer(blocks: &[Block]) -> Option<usize> {
    let floor = blocks
        .iter()
        .rposition(|block| {
            matches!(
                block,
                Block::CompactionBoundary(_) | Block::CompactionMarker(_)
            )
        })
        .map_or(0, |compaction| compaction + 1);
    let response = floor
        + blocks[floor..]
            .iter()
            .rposition(|block| matches!(block, Block::Response(_)))?;
    let before_answer = blocks[floor..response]
        .iter()
        .rposition(|block| !is_answer(block));
    Some(before_answer.map_or(floor, |index| floor + index + 1))
}

/// Whether a block is part of a model's answer.
fn is_answer(block: &Block) -> bool {
    matches!(
        block,
        Block::Thinking(_) | Block::RedactedThinking(_) | Block::Text(_) | Block::ToolCall(_)
    )
}

/// Whether a block opens an input turn (`runtime.md` § Block types): recovery
/// treats it as the start of its turn, and its `before_user` command-state
/// boundary is recorded.
pub(crate) fn opens_input_turn(block: &Block) -> bool {
    match block {
        Block::User(_) | Block::Context(_) => true,
        Block::Wakeup(wakeup) => wakeup.placement == WakeupPlacement::NewTurn,
        _ => false,
    }
}
