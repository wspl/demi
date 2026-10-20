//! The small requests of a user's conversations
//! (`crates/backend-providers/src/llm/small_model.rs`): each runs on the user's
//! small model, resolved for the
//! conversation from the catalogs of the user's entries and the user's
//! preferences when it starts, through the same metered runtime a turn uses,
//! so it counts in the usage ledger under its purpose and obeys the same
//! limits.

use std::rc::Rc;
use std::sync::Arc;

use demi_agent_server::ProviderResolver as _;
use demi_agent_server::small_request::{SmallRequest, SmallRequestError, run_small_request};
use demi_backend_database::StorageError;
use demi_backend_host_access::root_of;
use demi_backend_providers::llm::small_model::small_model;
use demi_shared_types::ModelSelection;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use tokio_util::sync::CancellationToken;

use super::providers::ConversationProviders;
use crate::services::Services;

/// Where a shard's small jobs send their requests.
pub(crate) struct SmallRequests {
    providers: Rc<ConversationProviders>,
    user: UserId,
    services: Arc<Services>,
}

/// Why a small request answered nothing.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SmallRequestFailure {
    #[error(transparent)]
    Provider(#[from] demi_agent_server::ResolveError),
    #[error(transparent)]
    Request(#[from] SmallRequestError),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl SmallRequests {
    pub(super) fn new(providers: Rc<ConversationProviders>, user: UserId, services: Arc<Services>) -> Self {
        Self {
            providers,
            user,
            services,
        }
    }

    /// The user's small model for a request of a conversation that infers
    /// with `conversation`, from the entries' catalogs as they are now.
    async fn resolve(&self, conversation: &ModelSelection) -> Result<ModelSelection, StorageError> {
        let services = &self.services;
        let owner = services.vault.owner_for(&self.user).await?;
        let entries = services.vault.entries(owner).await?;
        let preferences = services.control.preferences(self.user.clone()).await?;
        let catalog = services.assembly.model_catalog(&entries, false).await;
        Ok(small_model(&catalog, &preferences, conversation))
    }

    /// Runs `request` for the conversation `id`, which infers with
    /// `conversation`, on the small model, and answers the text of its
    /// response; none when `cancel` stopped it.
    pub(crate) async fn run(
        &self,
        id: &ConversationId,
        conversation: &ModelSelection,
        request: SmallRequest,
        cancel: CancellationToken,
    ) -> Result<Option<String>, SmallRequestFailure> {
        let selection = self.resolve(conversation).await?;
        let mut runtime = self.providers.runtime(&root_of(id), &selection).await?;
        let request_id = uuid::Uuid::new_v4().to_string();
        let answer = run_small_request(
            &mut *runtime,
            id.as_str(),
            request_id,
            &selection,
            request,
            cancel,
        )
        .await;
        runtime.close().await;
        Ok(answer?)
    }
}
