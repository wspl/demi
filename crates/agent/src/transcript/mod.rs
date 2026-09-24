//! A session's transcript (`runtime.md` § Transcript): its blocks, the
//! patches that record every change to them, the model's replay of them, and
//! the token estimates compaction decides by.

mod cut;
pub mod estimate;
mod journal;
mod log;
mod replay;

pub use cut::CutError;
pub(crate) use cut::{
    before_user, compaction_window, last_assistant_text, resume_point, rewind, through_assistant,
};
pub(crate) use journal::PatchBatch;
pub(crate) use log::TranscriptLog;
#[cfg(test)]
pub(crate) use replay::agent_message_envelope;
pub(crate) use replay::{replay, tool_input};

use demi_core::{Block, WakeupPlacement};

/// What the model receives for a `resume` block.
pub(crate) const RESUME_TEXT: &str = "Continue from where you left off.";

/// What the model receives for a fired yield wakeup.
pub(crate) const WAKEUP_TEXT: &str = "Scheduled yield wakeup fired. Continue the previous work and inspect any running command with shell_status when needed.";

/// What the error record of a turn the session was shut down under says.
pub(crate) const INTERRUPTED_TURN_MESSAGE: &str =
    "The agent session was shut down while this turn was running.";

/// The code of that record.
pub(crate) const INTERRUPTED_CODE: &str = "interrupted";

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
