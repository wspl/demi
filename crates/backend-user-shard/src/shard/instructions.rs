//! The user's personal instructions (`web-api.md` § Instructions), written
//! here once the edge checked the text. Each change reaches the user's pages
//! as the `instructions` part, and the next request of every node, since the
//! instructions source reads them at each request (`instructions.md` § When
//! it is sent).

use demi_backend_database::StorageError;
use demi_backend_page_sync::Part;

use super::Shard;

impl Shard {
    /// Replaces the user's personal instructions with `text`, which was
    /// checked; an empty text removes them.
    pub async fn set_instructions(&self, text: String) -> Result<(), StorageError> {
        self.services()
            .control
            .set_instructions(self.user().clone(), text)
            .await?;
        self.services().sync.mark(self.user(), Part::Instructions);
        Ok(())
    }
}
