//! The user's subagent settings (`web-api.md` § Subagents): the Subagent
//! switch and the profiles, written here once the edge checked a body's name
//! and texts. A profile's model is checked against its entry's catalog as a
//! conversation's model settings are, and is stored with its effort
//! explicit. Each change reaches the user's pages as the `subagents` part,
//! and the next spawn of every conversation reads it, since a spawn reads
//! the settings each time.

use demi_backend_accounts::subagents;
use demi_backend_database::StorageError;
use demi_backend_database::subagents::ProfileRefusal as Stored;
use demi_backend_host_access::transition::ChangeRefusal;
use demi_backend_page_sync::Part;
use demi_web_api_protocol::conversations::ModelSettings;
use demi_web_api_protocol::ids::ProfileId;
use demi_web_api_protocol::subagents::{NewProfile, ProfilePatch, SubagentProfile};

use super::Shard;

/// Why a profile was not written.
#[derive(Debug, thiserror::Error)]
pub enum ProfileRefusal {
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
    /// Turns subagents on or off for the user.
    pub async fn switch_subagents(&self, enabled: bool) -> Result<(), StorageError> {
        self.services()
            .control
            .set_subagents(self.user().clone(), enabled)
            .await?;
        self.mark_subagents();
        Ok(())
    }

    /// Creates the user's profile `profile`, whose name and texts were
    /// checked, enabled.
    pub async fn create_profile(
        &self,
        mut profile: NewProfile,
    ) -> Result<SubagentProfile, ProfileRefusal> {
        if let Some(model) = &profile.model {
            profile.model = Some(self.profile_model(model).await?);
        }
        let created = self
            .services()
            .control
            .create_profile(self.user().clone(), profile)
            .await??;
        self.mark_subagents();
        Ok(created)
    }

    /// Applies `patch`, whose name and texts were checked, to the user's
    /// profile `id`. Turning it on or off checks nothing else, so an
    /// unavailable profile can be disabled and enabled as it is.
    pub async fn patch_profile(
        &self,
        id: ProfileId,
        mut patch: ProfilePatch,
    ) -> Result<SubagentProfile, ProfileRefusal> {
        if let Some(Some(model)) = &patch.model {
            patch.model = Some(Some(self.profile_model(model).await?));
        }
        let changed = self
            .services()
            .control
            .patch_profile(self.user().clone(), id, move |stored| {
                subagents::merge(stored, patch)
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
    /// with the model's first effort when it names none.
    async fn profile_model(&self, model: &ModelSettings) -> Result<ModelSettings, ProfileRefusal> {
        let selection = self
            .chosen_selection(model)
            .await
            .map_err(ProfileRefusal::Model)?;
        Ok(ModelSettings {
            provider_id: model.provider_id.clone(),
            model_id: selection.model.id.clone(),
            thinking_effort: selection.thinking_effort().map(str::to_owned),
            service_tier_id: selection.service_tier_id.clone(),
        })
    }

    fn mark_subagents(&self) {
        self.services().sync.mark(self.user(), Part::Subagents);
    }
}
