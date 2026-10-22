//! A user's personal instructions: the rule a text follows before it is written.

use demi_shared_types::is_blank;

/// The personal instructions `text` stores as: itself, or empty for a blank
/// text, which removes them.
pub fn personal_instructions(text: String) -> String {
    if is_blank(&text) { String::new() } else { text }
}
