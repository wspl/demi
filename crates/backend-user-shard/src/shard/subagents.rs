//! The user's subagent settings (`web-api.md` § Subagents): the Subagents
//! switch, the limits and the profiles, written here for the settings page
//! and for `demi agent profiles` alike, after the same checks: a profile's
//! name and texts by the accounts service's rules, and its model against
//! its entry's catalog as a conversation's model settings are, stored with
//! its effort explicit. Each change reaches the user's pages as the
//! `subagents` part, and the next spawn of every conversation reads it,
//! since a spawn reads the settings each time.

use demi_backend_accounts::subagents::{self, ContextRefusal, ProfileTextError};
use demi_backend_database::StorageError;
use demi_backend_database::subagents::ProfileRefusal as Stored;
use demi_backend_host_access::transition::ChangeRefusal;
use demi_backend_page_sync::Part;
use demi_web_api_protocol::conversations::ModelSettings;
use demi_web_api_protocol::ids::ProfileId;
use demi_web_api_protocol::subagents::{NewProfile, ProfilePatch, SubagentProfile, SubagentsPatch};

use super::Shard;

/// Why a profile was not written.
#[derive(Debug, thiserror::Error)]
pub enum ProfileRefusal {
    /// A name or a text breaks its rule.
    #[error(transparent)]
    Text(#[from] ProfileTextError),
    /// The Context needs a model of the profile's own, or one its model
    /// offers.
    #[error(transparent)]
    Context(#[from] ContextRefusal),
    #[error(transparent)]
    Stored(#[from] Stored),
    /// The model is outside the user's scope, unlisted, or does not offer
    /// the effort or the tier.
    #[error(transparent)]
    Model(ChangeRefusal),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl Shard {
    /// Sets what `patch` names of the user's Subagents switch and limits.
    pub async fn patch_subagents(&self, patch: SubagentsPatch) -> Result<(), StorageError> {
        self.services()
            .control
            .set_subagents(self.user().clone(), patch)
            .await?;
        self.mark_subagents();
        Ok(())
    }

    /// Creates the user's profile `profile`, enabled or not.
    pub async fn create_profile(
        &self,
        mut profile: NewProfile,
        enabled: bool,
    ) -> Result<SubagentProfile, ProfileRefusal> {
        subagents::check_new(&profile)?;
        let checked = match &profile.model {
            Some(model) => Some(self.profile_model(model).await?),
            None => None,
        };
        subagents::check_context(
            checked.as_ref().map(|(model, window)| (model.model_id.as_str(), *window)),
            profile.context_limit,
        )?;
        profile.model = checked.map(|(model, _)| model);
        let created = self
            .services()
            .control
            .create_profile(self.user().clone(), profile, enabled)
            .await??;
        self.mark_subagents();
        Ok(created)
    }

    /// Applies `patch` to the user's profile `id`. Turning it on or off
    /// checks nothing else, so an unavailable profile can be disabled and
    /// enabled as it is.
    pub async fn patch_profile(
        &self,
        id: ProfileId,
        mut patch: ProfilePatch,
    ) -> Result<SubagentProfile, ProfileRefusal> {
        subagents::check_patch(&patch)?;
        // The model the profile has after the patch, with its window: the
        // patch's, or the stored one's when the patch names a Context alone.
        let model = match &patch.model {
            Some(Some(model)) => Some(self.profile_model(model).await?),
            Some(None) => None,
            None => match patch.context_limit {
                Some(Some(_)) => match self.stored_profile(&id).await?.model {
                    Some(stored) => Some(self.profile_model(&stored).await?),
                    None => None,
                },
                _ => None,
            },
        };
        if let Some(chosen) = patch.context_limit {
            subagents::check_context(
                model.as_ref().map(|(model, window)| (model.model_id.as_str(), *window)),
                chosen,
            )?;
        }
        let window = model.as_ref().map(|(_, window)| *window);
        if let Some(Some(_)) = patch.model {
            patch.model = Some(model.map(|(model, _)| model));
        }
        let changed = self
            .services()
            .control
            .patch_profile(self.user().clone(), id, move |stored| {
                subagents::merge(stored, patch, window)
            })
            .await??;
        self.mark_subagents();
        Ok(changed)
    }

    /// Deletes the user's profile `id`.
    pub async fn delete_profile(&self, id: ProfileId) -> Result<(), ProfileRefusal> {
        let deleted = self
            .services()
            .control
            .delete_profile(self.user().clone(), id)
            .await?;
        if !deleted {
            return Err(Stored::NotFound.into());
        }
        self.mark_subagents();
        Ok(())
    }

    /// `model` as a profile stores it, checked against the entry's catalog,
    /// with the model's first effort when it names none, and the model's
    /// context window, which a profile's Context is checked against.
    pub(crate) async fn profile_model(
        &self,
        model: &ModelSettings,
    ) -> Result<(ModelSettings, u32), ProfileRefusal> {
        let selection = self
            .chosen_selection(model)
            .await
            .map_err(ProfileRefusal::Model)?;
        let settings = ModelSettings {
            provider_id: model.provider_id.clone(),
            model_id: selection.model.id.clone(),
            thinking_effort: selection.thinking_effort().map(str::to_owned),
            service_tier_id: selection.service_tier_id.clone(),
        };
        Ok((settings, selection.model.context_window))
    }

    /// The user's profile `id` as stored.
    async fn stored_profile(&self, id: &ProfileId) -> Result<SubagentProfile, ProfileRefusal> {
        let settings = self
            .services()
            .control
            .subagent_settings(self.user().clone())
            .await?;
        settings
            .profiles
            .into_iter()
            .find(|profile| profile.id == *id)
            .ok_or(Stored::NotFound.into())
    }

    fn mark_subagents(&self) {
        self.services().sync.mark(self.user(), Part::Subagents);
    }
}
