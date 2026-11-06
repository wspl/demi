//! The user's personal instructions: the rule a text follows before it is
//! written, and their writes.
//!
//! Each change reaches the user's pages as the `instructions` part, and the next
//! request of every node, since the instructions source reads them at each
//! request.

use demi_core_store::StorageError;
use demi_core_sync::Part;
use demi_shared_types::is_blank;

use crate::Core;

/// The personal instructions `text` stores as: itself, or empty for a blank
/// text, which removes them.
pub fn personal_instructions(text: String) -> String {
    if is_blank(&text) { String::new() } else { text }
}

impl Core {
    /// Replaces the user's personal instructions with `text`, which was
    /// checked; an empty text removes them.
    pub async fn set_instructions(&self, text: String) -> Result<(), StorageError> {
        self.store().set_instructions(text)
            .await?;
        self.services().sync.mark(self.user(), Part::Instructions);
        Ok(())
    }
}
