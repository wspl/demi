//! What the profile changes of `demi agent profiles` need of the user's
//! shard (`subagents.md` § Managing profiles): a command's change resolved
//! into the body the settings page sends, written by the same path with the
//! same checks and the same words; and the user's entries with their
//! catalogs, which `demi providers` lists too.

use demi_agent_tools::{
    EFFORT_OFF, ModelChoice, ModelEdit, NewProfileDraft, ProfileEdit, TIER_DEFAULT,
};
use demi_backend_database::StorageError;
use demi_backend_providers::vault::entries::ProviderEntry;
use demi_shared_types::THINKING_OFF;
use demi_web_api_protocol::conversations::ModelSettings;
use demi_web_api_protocol::ids::ProviderId;
use demi_web_api_protocol::providers::{Availability, CatalogProvider};
use demi_web_api_protocol::settings::Preferences;
use demi_web_api_protocol::subagents::{NewProfile, ProfilePatch, SubagentProfile};

use super::Shard;

/// The user's provider entries with their catalogs, and the preferences
/// that hide some of them, read together.
pub(crate) struct Scope {
    pub(crate) entries: Vec<ProviderEntry>,
    pub(crate) catalog: Vec<CatalogProvider>,
    pub(crate) preferences: Preferences,
}

impl Scope {
    /// Whether the menu offers the entry of `catalog`: the user has it on
    /// and its models can be used now.
    fn offers(&self, catalog: &CatalogProvider) -> bool {
        !self.preferences.hides_provider(&catalog.provider_id)
            && matches!(catalog.availability, Availability::Available {})
    }

    /// The catalog of `entry`, read with the entries.
    pub(crate) fn catalog_of(&self, entry: &ProviderEntry) -> Option<&CatalogProvider> {
        self.catalog
            .iter()
            .find(|catalog| catalog.provider_id == entry.id)
    }

    fn entry(&self, catalog: &CatalogProvider) -> Option<&ProviderEntry> {
        self.entries
            .iter()
            .find(|entry| entry.id == catalog.provider_id)
    }
}

impl Shard {
    /// Creates the profile a `create` command gives.
    pub(crate) async fn create_drafted_profile(
        &self,
        draft: NewProfileDraft,
    ) -> Result<SubagentProfile, String> {
        let model = match draft.model {
            Some(choice) => Some(self.chosen_model(choice, None).await?),
            None => None,
        };
        let profile = NewProfile {
            name: draft.name,
            description: draft.description,
            model,
            instructions: draft.instructions,
            can_spawn: draft.can_spawn,
        };
        self.create_profile(profile, draft.enabled)
            .await
            .map_err(|refusal| refusal.to_string())
    }

    /// Applies the change an `edit` command gives to the profile `name`.
    pub(crate) async fn edit_named_profile(
        &self,
        name: &str,
        edit: ProfileEdit,
    ) -> Result<SubagentProfile, String> {
        let stored = self.named_profile(name).await?;
        let model = match edit.model {
            None => None,
            Some(ModelEdit::Parent) => Some(None),
            Some(ModelEdit::Choose(choice)) => {
                Some(Some(self.chosen_model(choice, stored.model.as_ref()).await?))
            }
        };
        let patch = ProfilePatch {
            name: edit.name,
            description: edit.description,
            model,
            instructions: edit.instructions,
            can_spawn: edit.can_spawn,
            enabled: edit.enabled,
        };
        self.patch_profile(stored.id, patch)
            .await
            .map_err(|refusal| refusal.to_string())
    }

    /// Deletes the profile `name`.
    pub(crate) async fn delete_named_profile(&self, name: &str) -> Result<(), String> {
        let stored = self.named_profile(name).await?;
        self.delete_profile(stored.id)
            .await
            .map_err(|refusal| refusal.to_string())
    }

    /// The user's profile `name`, enabled or not.
    async fn named_profile(&self, name: &str) -> Result<SubagentProfile, String> {
        let settings = self
            .services()
            .control
            .subagent_settings(self.user().clone())
            .await
            .map_err(|error| error.to_string())?;
        settings
            .profiles
            .into_iter()
            .find(|profile| profile.name == name)
            .ok_or_else(|| format!("No such subagent profile \"{name}\""))
    }

    /// The model settings `choice` names: of the model it names, which the
    /// menu offers, or of `current`, the profile's own, when it names only
    /// an effort or a tier. The save path then checks them against the
    /// entry's catalog as it checks the settings page's.
    async fn chosen_model(
        &self,
        choice: ModelChoice,
        current: Option<&ModelSettings>,
    ) -> Result<ModelSettings, String> {
        let effort = choice.effort.map(|effort| {
            if effort == EFFORT_OFF {
                THINKING_OFF.to_owned()
            } else {
                effort
            }
        });
        let tier = choice.tier.map(|tier| (tier != TIER_DEFAULT).then_some(tier));
        let Some(model) = choice.model else {
            let current = current.ok_or(
                "the profile inherits the parent's model; give --model with --effort or --tier",
            )?;
            return Ok(ModelSettings {
                thinking_effort: effort.or_else(|| current.thinking_effort.clone()),
                service_tier_id: tier.unwrap_or_else(|| current.service_tier_id.clone()),
                ..current.clone()
            });
        };
        let scope = self.scope().await.map_err(|error| error.to_string())?;
        let provider = match choice.provider {
            Some(named) => named_entry(&scope, &named, &model)?,
            None => offering_entry(&scope, &model)?,
        };
        Ok(ModelSettings {
            provider_id: provider,
            model_id: model,
            thinking_effort: effort,
            service_tier_id: tier.flatten(),
        })
    }

    /// The user's provider entries, their catalogs and preferences now.
    pub(crate) async fn scope(&self) -> Result<Scope, StorageError> {
        let services = self.services();
        let owner = services.vault.owner_for(self.user()).await?;
        let entries = services.vault.entries(owner).await?;
        let catalog = services.assembly.model_catalog(&entries, false).await;
        let preferences = services.control.preferences(self.user().clone()).await?;
        Ok(Scope {
            entries,
            catalog,
            preferences,
        })
    }
}

/// The entry `named`, by its name or its id, that a profile may take
/// `model` of: one the user has on, whose model `model` the user has on.
fn named_entry(
    scope: &Scope,
    named: &str,
    model: &str,
) -> Result<ProviderId, String> {
    let entry = super::provider_commands::named(&scope.entries, named)?;
    if scope.preferences.hides_provider(&entry.id) {
        return Err(hidden_entry(&entry.label));
    }
    if scope.preferences.hides_model(&entry.id, model) {
        return Err(hidden_model(model, &entry.label));
    }
    Ok(entry.id.clone())
}

/// The one entry of the menu that offers `model`.
fn offering_entry(
    scope: &Scope,
    model: &str,
) -> Result<ProviderId, String> {
    let listing: Vec<&CatalogProvider> = scope
        .catalog
        .iter()
        .filter(|catalog| catalog.models.iter().any(|listed| listed.model.id == model))
        .collect();
    let offered: Vec<&CatalogProvider> = listing
        .iter()
        .copied()
        .filter(|catalog| {
            scope.offers(catalog) && !scope.preferences.hides_model(&catalog.provider_id, model)
        })
        .collect();
    match offered.as_slice() {
        [catalog] => Ok(catalog.provider_id.clone()),
        [] => {
            let label = |catalog: &CatalogProvider| {
                scope
                    .entry(catalog)
                    .map_or_else(|| catalog.display_name.clone(), |entry| entry.label.clone())
            };
            // The model the user turned off in an entry that is on says
            // more than an entry turned off as a whole.
            let hides = |catalog: &CatalogProvider| {
                scope.preferences.hides_provider(&catalog.provider_id)
            };
            if let Some(catalog) = listing.iter().find(|catalog| {
                !hides(catalog) && scope.preferences.hides_model(&catalog.provider_id, model)
            }) {
                return Err(hidden_model(model, &label(catalog)));
            }
            if let Some(catalog) = listing.iter().find(|catalog| hides(catalog)) {
                return Err(hidden_entry(&label(catalog)));
            }
            if !listing.is_empty() {
                let entries: Vec<String> = listing.iter().map(|catalog| label(catalog)).collect();
                return Err(format!(
                    "model \"{model}\" is offered only by provider entries that cannot run now: {}; `demi providers list` lists the models the user can use",
                    entries.join(", ")
                ));
            }
            Err(format!(
                "no provider entry offers the model \"{model}\"; `demi providers list` lists the models the user can use"
            ))
        }
        several => {
            let entries: Vec<String> = several
                .iter()
                .map(|catalog| match scope.entry(catalog) {
                    Some(entry) => format!("{} ({})", entry.label, entry.id),
                    None => catalog.provider_id.as_str().to_owned(),
                })
                .collect();
            Err(format!(
                "several provider entries offer the model \"{model}\": {}; name one with --provider",
                entries.join(", ")
            ))
        }
    }
}

/// Why a profile cannot take a model of an entry the user turned off.
fn hidden_entry(entry: &str) -> String {
    format!(
        "the provider entry \"{entry}\" is turned off in Settings > Providers; choose a model `demi providers list` lists, or ask the user to turn the entry on"
    )
}

/// Why a profile cannot take a model the user turned off.
fn hidden_model(model: &str, entry: &str) -> String {
    format!(
        "the model \"{model}\" of \"{entry}\" is turned off in Settings > Providers; choose a model `demi providers list` lists, or ask the user to turn it on"
    )
}
