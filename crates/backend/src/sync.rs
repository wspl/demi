//! The reconstructible snapshot the page polls (`backend.md` § Browser
//! synchronization). The user's shard assembles it from the stores and its
//! own state; it is not one atomic read across databases, and a later poll
//! catches what changed during one.

use demi_web_api::auth::UserDto;
use demi_web_api::providers::{ProviderReading, ProviderState};
use demi_web_api::state::ProductState;
use futures_util::future::join_all;

use crate::shard::Shard;
use crate::storage::StorageError;

impl Shard {
    /// The snapshot for `user`, this shard's user as the session gate
    /// resolved them: with the entries the user infers with, each with what
    /// its provider says now, which a user who only infers sees without
    /// accounts, plan or usage. One entry that cannot be read leaves the
    /// others intact.
    pub(crate) async fn product_state(&self, user: UserDto) -> Result<ProductState, StorageError> {
        let services = self.services();
        let preferences = services.control.preferences(self.user().clone()).await?;
        let owner = services.vault.owner_for(&user.id).await?;
        let entries = services.vault.entries(owner).await?;
        let disclose = services.vault.configures(&user);
        let providers = join_all(entries.iter().map(|entry| async move {
            let details = match services.assembly.details(entry, disclose).await {
                Ok(details) => ProviderReading::Read(Box::new(details)),
                Err(error) => ProviderReading::Failed {
                    message: error.to_string(),
                },
            };
            ProviderState {
                provider: entry.dto(),
                details,
            }
        }))
        .await;
        let workspaces = services.control.workspaces(self.user().clone()).await?;
        let devices = self.device_list().await?;
        let mut conversations = self.conversation_summaries(false).await?;
        conversations.extend(self.conversation_summaries(true).await?);
        Ok(ProductState {
            user,
            mode: services.mode,
            preferences,
            providers,
            workspaces: workspaces.into_iter().map(|workspace| workspace.dto()).collect(),
            devices,
            conversations,
        })
    }
}
