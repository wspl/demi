//! What a subagent's round runs on, in the names the model menu shows now
//! (`subagents.md` § What an agent runs on): the provider entry's name from
//! the user's entries, and the model's name and Fast tier from the catalog
//! the menu reads. The `subagent` frames and the conversation's subagents
//! route both read it here.

use demi_agent_tools::runs_on as job_model;
use demi_conversation_socket_protocol::JobModel;
use demi_shared_types::ModelSelection;
use demi_web_api_protocol::ids::UserId;

use crate::services::Services;

/// What rounds on `selections` of `user`'s run on, in their order. An
/// entry or a model the user's scope no longer has, or a scope that cannot
/// be read, shows by what the selection holds ([`job_model`]).
pub async fn runs_on(
    services: &Services,
    user: &UserId,
    selections: &[ModelSelection],
) -> Vec<JobModel> {
    let entries = match services.vault.owner_for(user).await {
        Ok(owner) => services.vault.entries(owner).await,
        Err(error) => Err(error),
    };
    let entries = entries.unwrap_or_else(|error| {
        // The names only fall back to the selection's; the job still shows.
        tracing::warn!(%error, "the provider entries were not read for a subagent's model");
        Vec::new()
    });
    let catalog = services.assembly.model_catalog(&entries, false).await;
    selections
        .iter()
        .map(|selection| {
            let provider = catalog
                .iter()
                .find(|provider| provider.provider_id.as_str() == selection.provider_id);
            let model = provider.and_then(|provider| {
                provider
                    .models
                    .iter()
                    .find(|model| model.model.id == selection.model.id)
            });
            job_model(
                selection,
                provider.map(|provider| provider.display_name.as_str()),
                model.map(|model| &model.model),
            )
        })
        .collect()
}
