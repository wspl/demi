//! A session's transcript (`transcript.md` § Transcript): its blocks, the
//! patches that record every change to them, the identities it gives blocks
//! and turns, the points it is cut at, the model's replay of them, the token
//! estimates compaction decides by.

mod compaction;
mod cut;
pub mod estimate;
mod ids;
mod journal;
mod log;
mod replay;
mod reports;
#[cfg(feature = "testing")]
pub mod testing;

pub use compaction::{
    COMPACTION_NOTE, CompactionWindow, answered_ends, compaction_window, kept_messages,
    summary_text,
};
pub use cut::{
    CutError, ForkEnd, ResumePoint, Rewind, before_user, ended_turn, ended_turn_answer, fork_prefix, last_answer, last_failure,
    last_assistant_text, last_turn, resume_point, rewind, turn_record, unwind,
};
pub use ids::{IdSource, RandomIds};
pub use journal::{DirtyRows, PatchBatch};
pub use log::{PendingCall, TranscriptLog, kept};
pub use replay::{REPLAY_CHARS, Replay, ReplayedBlock, RequestView, replay, tool_input};
pub use reports::{duration, labelled_output, report_text, reports_text};

use demi_shared_types::{Block, GoneCause, ModelMediaKind, WakeupPlacement};

/// What the model receives for a tool's medium that is gone (`transcript.md`
/// § Media): `[<kind> not stored: <reason>]`.
pub(crate) fn gone_text(kind: ModelMediaKind, cause: &GoneCause) -> String {
    let kind = kind.name();
    match cause {
        GoneCause::NotStored { error } => format!("[{kind} not stored: {error}]"),
    }
}

/// What the error record of a turn the session was shut down under says.
pub const INTERRUPTED_TURN_MESSAGE: &str =
    "The agent session was shut down while this turn was running.";

/// The code of that record.
pub const INTERRUPTED_CODE: &str = "interrupted";

/// Whether `block` is the record that says why its turn is unfinished after
/// a shutdown or crash.
pub fn is_interruption(block: &Block) -> bool {
    matches!(block, Block::Error(error) if error.code.as_deref() == Some(INTERRUPTED_CODE))
}

/// Where replay starts in `blocks` (`transcript.md` § Replay): at the last
/// `compaction_boundary`, or at the first block when there is none.
pub fn replay_start(blocks: &[Block]) -> usize {
    blocks
        .iter()
        .rposition(|block| matches!(block, Block::CompactionBoundary(_)))
        .unwrap_or(0)
}

/// Where the answer to the session's latest answered request begins
/// (`transcript.md` § Replay): that request's `response` block comes last,
/// after the last `compaction_marker`, and its answer is the run of
/// thinking, text and tool-call blocks directly before that `response`
/// block. What the blocks before the answer replay is what the request
/// carried. None when no request was answered since the last compaction.
/// A boundary whose marker an edit cut counts as the last compaction.
pub(crate) fn latest_answer(blocks: &[Block]) -> Option<usize> {
    answered_ends(blocks).last().copied()
}

/// Whether a block is a compaction's boundary or marker.
pub(crate) fn is_compaction(block: &Block) -> bool {
    matches!(
        block,
        Block::CompactionBoundary(_) | Block::CompactionMarker(_)
    )
}

/// Whether a block is part of a model's answer.
pub(crate) fn is_answer(block: &Block) -> bool {
    matches!(
        block,
        Block::Thinking(_) | Block::RedactedThinking(_) | Block::Text(_) | Block::ToolCall(_)
    )
}

/// Whether a block opens an input turn (`transcript.md` § Block types): recovery
/// treats it as the start of its turn.
pub fn opens_input_turn(block: &Block) -> bool {
    match block {
        Block::User(_) | Block::Context(_) => true,
        Block::Wakeup(wakeup) => wakeup.placement == WakeupPlacement::NewTurn,
        _ => false,
    }
}
