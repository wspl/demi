//! A conversation's model settings (`models.md` § A conversation's model
//! settings; `web-api.md` § Sidebar mutations, read state and page
//! synchronization). A patch names the parts it changes, and the
//! conversation's settings order applies one change at a time to the
//! selection the record holds: the change is checked against the entry's
//! catalog, a live tree gets the runtime a new provider entry needs, the
//! record commits, and then the live tree switches. A crash between the
//! commit and the switch leaves the change in the record, which the tree
//! takes when it next opens.

use demi_agent::ResolveError;
use demi_backend_storage::conversation_index::{RecordChange, SettingsChange};
use demi_core::{ModelSelection, ProviderModel};
use demi_web_api::ids::{ConversationId, ProviderId};

use demi_backend_host_access::root_of;
use demi_backend_host_access::transition::ChangeRefusal;
use crate::shard::Shard;

impl Shard {
    /// Applies `change` to the model settings of the user's conversation
    /// `id`. The caller holds the conversation's settings order, so the
    /// record read here is the one the change applies to.
    pub(super) async fn change_settings(
        &self,
        id: &ConversationId,
        change: SettingsChange,
    ) -> Result<(), ChangeRefusal> {
        let record = self
            .services()
            .control
            .conversation(id.clone())
            .await?
            .ok_or(ChangeRefusal::NotFound)?;
        if record.archived {
            return Err(ChangeRefusal::Archived);
        }
        let selection = self
            .settings_selection(record.model.as_ref(), change)
            .await?;
        let root = root_of(id);
        let switch = self
            .agent()
            .prepare_switch(&root, selection.clone())
            .await
            .map_err(runtime_refused)?;
        let committed = self.host_shard().commit(id, RecordChange::Model(selection)).await;
        if let Some(switch) = switch {
            if committed.is_ok() {
                self.agent().switch_model(&root, switch).await;
            } else {
                switch.discard().await;
            }
        }
        committed
    }

    /// The selection `change` makes of the conversation's selection
    /// `current`: a switch takes the new model's facts from the entry's
    /// catalog with the parts it names, and null for the others; a change of
    /// parts keeps the model's facts and changes those parts. Each part it
    /// names is one the catalog's model offers.
    async fn settings_selection(
        &self,
        current: Option<&ModelSelection>,
        change: SettingsChange,
    ) -> Result<ModelSelection, ChangeRefusal> {
        let Some(choice) = change.model else {
            let current = current.ok_or(ChangeRefusal::ModelNotSelected)?;
            let provider = ProviderId::try_from(current.provider_id.as_str())
                .map_err(|_| ChangeRefusal::ProviderNotFound)?;
            let listed = self.listed_model(&provider, &current.model.id).await?;
            let mut selection = current.clone();
            if let Some(effort) = change.thinking_effort {
                selection.thinking = listed.thinking_for(effort.as_deref())?;
            }
            if let Some(tier) = change.service_tier_id {
                selection.service_tier_id = listed.tier_for(tier.as_deref())?;
            }
            return Ok(selection);
        };
        let listed = self
            .listed_model(&choice.provider_id, &choice.model_id)
            .await?;
        let effort = change.thinking_effort.flatten();
        let tier = change.service_tier_id.flatten();
        let thinking = listed.thinking_for(effort.as_deref())?;
        let tier = listed.tier_for(tier.as_deref())?;
        Ok(listed.selection(choice.provider_id.as_str(), thinking, tier))
    }

    /// The model `model` of the user's entry `provider`, as the entry's
    /// catalog lists it now.
    async fn listed_model(
        &self,
        provider: &ProviderId,
        model: &str,
    ) -> Result<ProviderModel, ChangeRefusal> {
        let services = self.services();
        let entry = services
            .vault
            .visible(self.user(), provider)
            .await?
            .ok_or(ChangeRefusal::ProviderNotFound)?;
        let built = services.assembly.provider_for(&entry).await;
        let catalog = services.assembly.entry_catalog(&entry, &built, false).await;
        catalog
            .models
            .into_iter()
            .find(|listed| listed.id == model)
            .ok_or(ChangeRefusal::ModelNotFound)
    }
}

/// A runtime the live tree could not get for the new selection: an entry
/// outside the user's scope, or one that cannot run.
fn runtime_refused(error: ResolveError) -> ChangeRefusal {
    match error {
        ResolveError::Unknown(_) => ChangeRefusal::ProviderNotFound,
        ResolveError::Failed(message) => ChangeRefusal::Runtime(message),
    }
}
