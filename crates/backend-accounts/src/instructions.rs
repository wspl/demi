//! A user's personal instructions (`web-api.md` § Instructions): the rule a
//! text follows before it is written.

use demi_shared_types::is_blank;

use crate::subagents::INSTRUCTIONS_MAX;

/// Why a text is not personal instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("text: must be at most {INSTRUCTIONS_MAX} characters")]
pub struct InstructionsTooLong;

/// The personal instructions `text` stores as: itself, or empty for a blank
/// text, which removes them.
pub fn personal_instructions(text: String) -> Result<String, InstructionsTooLong> {
    if text.chars().count() > INSTRUCTIONS_MAX {
        return Err(InstructionsTooLong);
    }
    Ok(if is_blank(&text) { String::new() } else { text })
}
