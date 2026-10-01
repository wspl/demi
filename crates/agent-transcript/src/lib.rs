//! A session's transcript (`runtime.md` § Transcript): its blocks, the
//! patches that record every change to them, the identities it gives blocks
//! and turns, the points it is cut at, the model's replay of them, the token
//! estimates compaction decides by, and the retirement of a tool result's
//! expired media.

mod cut;
pub mod estimate;
mod ids;
mod journal;
mod log;
mod replay;
pub mod retire;
#[cfg(feature = "testing")]
pub mod testing;

pub use cut::{
    CompactionWindow, CutError, ResumePoint, Rewind, before_user, compaction_window,
    last_assistant_text, resume_point, rewind, through_assistant,
};
pub use ids::{IdSource, RandomIds};
pub use journal::{DirtyRows, PatchBatch};
pub use log::{PendingCall, TranscriptLog};
pub use replay::{REPLAY_CHARS, Replay, RequestView, replay, tool_input};

use demi_shared_types::{Block, GoneCause, ModelMediaKind, WakeupPlacement};
use jiff::tz::TimeZone;

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

/// What the error record of a turn the session was shut down under says.
pub const INTERRUPTED_TURN_MESSAGE: &str =
    "The agent session was shut down while this turn was running.";

/// The code of that record.
pub const INTERRUPTED_CODE: &str = "interrupted";

/// Where replay starts in `blocks` (`runtime.md` § Replay): at the last
/// `compaction_boundary`, or at the first block when there is none.
pub fn replay_start(blocks: &[Block]) -> usize {
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
pub fn opens_input_turn(block: &Block) -> bool {
    match block {
        Block::User(_) | Block::Context(_) => true,
        Block::Wakeup(wakeup) => wakeup.placement == WakeupPlacement::NewTurn,
        _ => false,
    }
}
