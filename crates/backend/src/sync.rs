//! The reconstructible snapshot the page polls (`backend.md` § Browser
//! synchronization). The user's shard assembles it from the stores and its
//! own state; it is not one atomic read across databases, and a later poll
//! catches what changed during one.

use demi_web_api::auth::UserDto;
use demi_web_api::state::ProductState;

use crate::shard::Shard;
use crate::storage::StorageError;

impl Shard {
    /// The snapshot for `user`, this shard's user as the session gate
    /// resolved them.
    pub(crate) async fn product_state(&self, user: UserDto) -> Result<ProductState, StorageError> {
        let services = self.services();
        let preferences = services.control.preferences(self.user().clone()).await?;
        Ok(ProductState {
            user,
            mode: services.mode,
            preferences,
        })
    }
}
